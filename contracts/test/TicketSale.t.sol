// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.27;

import {Test} from "forge-std/Test.sol";
import {PausableUpgradeable} from "openzeppelin-contracts-upgradeable/contracts/utils/PausableUpgradeable.sol";
import {ProxyAdmin} from "openzeppelin-contracts/contracts/proxy/transparent/ProxyAdmin.sol";
import {
    ITransparentUpgradeableProxy
} from "openzeppelin-contracts/contracts/proxy/transparent/TransparentUpgradeableProxy.sol";
import {UnsafeUpgrades} from "openzeppelin-foundry-upgrades/Upgrades.sol";
import {TicketSale} from "../src/TicketSale.sol";
import {IntentSigning} from "./mocks/IntentSigning.sol";
import {MockERC20} from "./mocks/MockERC20.sol";
import {TicketSaleV2Mock} from "./mocks/TicketSaleV2Mock.sol";

interface ITicketSaleV2 {
    function version() external view returns (uint256);
}

contract TicketSaleTest is Test {
    address internal constant BUYER = address(0xBEEF);
    address internal constant TREASURY = address(0xCAFE);
    address internal constant OTHER_TOKEN = address(0x1234);
    address internal constant PROXY_ADMIN_OWNER = address(0xA11CE);
    uint256 internal constant PURCHASE_SIGNER_PK = uint256(keccak256("purchase-signer"));
    bytes32 internal constant ERC1967_ADMIN_SLOT = bytes32(uint256(keccak256("eip1967.proxy.admin")) - 1);

    TicketSale internal sale;
    address internal proxy;
    MockERC20 internal usdt;
    MockERC20 internal usdc;

    function setUp() public {
        usdt = new MockERC20("Tether USD", "USDT", 6);
        usdc = new MockERC20("USD Coin", "USDC", 6);

        address[] memory payment_tokens = new address[](2);
        payment_tokens[0] = address(usdt);
        payment_tokens[1] = address(usdc);

        TicketSale implementation = new TicketSale();
        bytes memory init_data =
            abi.encodeCall(TicketSale.initialize, (address(this), address(this), TREASURY, payment_tokens));
        proxy = UnsafeUpgrades.deployTransparentProxy(address(implementation), PROXY_ADMIN_OWNER, init_data);
        sale = TicketSale(proxy);

        uint64[] memory level_1_times = new uint64[](2);
        uint256[] memory level_1_prices = new uint256[](2);
        level_1_times[0] = 1_000;
        level_1_times[1] = 2_000;
        level_1_prices[0] = 100e6;
        level_1_prices[1] = 80e6;
        sale.setPriceSchedule(1, level_1_times, level_1_prices);

        uint64[] memory level_2_times = new uint64[](2);
        uint256[] memory level_2_prices = new uint256[](2);
        level_2_times[0] = 1_000;
        level_2_times[1] = 2_000;
        level_2_prices[0] = 200e6;
        level_2_prices[1] = 150e6;
        sale.setPriceSchedule(2, level_2_times, level_2_prices);

        uint64[] memory level_3_times = new uint64[](2);
        uint256[] memory level_3_prices = new uint256[](2);
        level_3_times[0] = 1_000;
        level_3_times[1] = 2_000;
        level_3_prices[0] = 300e6;
        level_3_prices[1] = 250e6;
        sale.setPriceSchedule(3, level_3_times, level_3_prices);

        usdt.mint(BUYER, 1_000_000e6);
        vm.prank(BUYER);
        usdt.approve(address(sale), type(uint256).max);
    }

    function test_Quote_UsesCurrentSlotPrice() public {
        vm.warp(1_500);

        uint8[] memory levels = new uint8[](2);
        uint256[] memory quantities = new uint256[](2);
        levels[0] = 1;
        levels[1] = 3;
        quantities[0] = 2;
        quantities[1] = 1;

        (uint256 total_amount, uint256[] memory unit_prices) = sale.quote(levels, quantities);

        assertEq(unit_prices[0], 100e6);
        assertEq(unit_prices[1], 300e6);
        assertEq(total_amount, 500e6);
    }

    function test_Quote_AfterSlotShift_UsesNewPrice() public {
        vm.warp(2_100);

        uint8[] memory levels = new uint8[](1);
        uint256[] memory quantities = new uint256[](1);
        levels[0] = 2;
        quantities[0] = 2;

        (uint256 total_amount, uint256[] memory unit_prices) = sale.quote(levels, quantities);

        assertEq(unit_prices[0], 150e6);
        assertEq(total_amount, 300e6);
    }

    function test_Purchase_TransfersFundsAndAdvancesOrderId() public {
        vm.warp(1_500);

        uint8[] memory levels = new uint8[](2);
        uint256[] memory quantities = new uint256[](2);
        levels[0] = 1;
        levels[1] = 2;
        quantities[0] = 1;
        quantities[1] = 3;

        vm.prank(BUYER);
        (uint256 order_id, uint256 total_amount) = sale.purchase(address(usdt), levels, quantities);

        assertEq(order_id, 1);
        assertEq(total_amount, 700e6);
        assertEq(usdt.balanceOf(TREASURY), 700e6);
        assertEq(sale.next_order_id(), 2);
    }

    function test_Purchase_RevertIfUnsupportedToken() public {
        vm.warp(1_500);

        uint8[] memory levels = new uint8[](1);
        uint256[] memory quantities = new uint256[](1);
        levels[0] = 1;
        quantities[0] = 1;

        vm.prank(BUYER);
        vm.expectRevert(abi.encodeWithSelector(TicketSale.UnsupportedPaymentToken.selector, OTHER_TOKEN));
        sale.purchase(OTHER_TOKEN, levels, quantities);
    }

    function test_Purchase_RevertIfPaused() public {
        sale.pause();
        vm.warp(1_500);

        uint8[] memory levels = new uint8[](1);
        uint256[] memory quantities = new uint256[](1);
        levels[0] = 1;
        quantities[0] = 1;

        vm.prank(BUYER);
        vm.expectRevert(PausableUpgradeable.EnforcedPause.selector);
        sale.purchase(address(usdt), levels, quantities);
    }

    function test_PurchaseWithAuthorization_UsesDiscountedAmountAndMarksIntentConsumed() public {
        vm.warp(1_500);

        address purchase_signer = vm.addr(PURCHASE_SIGNER_PK);
        sale.setPurchaseSigner(purchase_signer);

        uint8[] memory levels = new uint8[](2);
        uint256[] memory quantities = new uint256[](2);
        levels[0] = 1;
        levels[1] = 2;
        quantities[0] = 1;
        quantities[1] = 2;

        bytes32 intent_id = keccak256("intent-1");
        uint256 final_total_amount = 450e6;
        uint64 expires_at = uint64(block.timestamp + 15 minutes);

        bytes32 digest = IntentSigning.digest(
            address(sale),
            block.chainid,
            BUYER,
            address(usdt),
            levels,
            quantities,
            intent_id,
            final_total_amount,
            expires_at
        );
        bytes32 signed_digest = IntentSigning.ethSignedDigest(digest);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(PURCHASE_SIGNER_PK, signed_digest);
        bytes memory signature = abi.encodePacked(r, s, v);

        vm.prank(BUYER);
        (uint256 order_id, uint256 charged_amount) = sale.purchaseWithAuthorization(
            address(usdt),
            levels,
            quantities,
            intent_id,
            final_total_amount,
            expires_at,
            signature
        );

        assertEq(order_id, 1);
        assertEq(charged_amount, final_total_amount);
        assertEq(usdt.balanceOf(TREASURY), final_total_amount);
        assertTrue(sale.consumed_intents(intent_id));

        vm.prank(BUYER);
        vm.expectRevert(
            abi.encodeWithSelector(
                TicketSale.IntentAlreadyConsumed.selector,
                intent_id
            )
        );
        sale.purchaseWithAuthorization(
            address(usdt),
            levels,
            quantities,
            intent_id,
            final_total_amount,
            expires_at,
            signature
        );
    }

    function test_CurrentPrice_RevertBeforeScheduleStart() public {
        vm.warp(999);
        vm.expectRevert(abi.encodeWithSelector(TicketSale.PriceNotStarted.selector, uint8(1), uint64(999)));
        sale.currentPrice(1);
    }

    function test_Upgrade_PreservesStateAndAddsNewLogic() public {
        vm.warp(1_500);

        uint8[] memory levels = new uint8[](1);
        uint256[] memory quantities = new uint256[](1);
        levels[0] = 1;
        quantities[0] = 2;

        vm.prank(BUYER);
        (uint256 order_id_before, uint256 total_amount_before) = sale.purchase(address(usdt), levels, quantities);
        assertEq(order_id_before, 1);
        assertEq(total_amount_before, 200e6);
        assertEq(sale.next_order_id(), 2);

        TicketSaleV2Mock upgraded_implementation = new TicketSaleV2Mock();
        ProxyAdmin proxy_admin = ProxyAdmin(_proxyAdminAddress(proxy));
        vm.prank(PROXY_ADMIN_OWNER);
        proxy_admin.upgradeAndCall(
            ITransparentUpgradeableProxy(payable(proxy)), address(upgraded_implementation), bytes("")
        );

        assertEq(sale.next_order_id(), 2);
        assertEq(ITicketSaleV2(address(sale)).version(), 2);

        vm.prank(BUYER);
        (uint256 order_id_after, uint256 total_amount_after) = sale.purchase(address(usdt), levels, quantities);
        assertEq(order_id_after, 2);
        assertEq(total_amount_after, 200e6);
    }

    function _proxyAdminAddress(address ticket_sale_proxy) internal view returns (address) {
        return address(uint160(uint256(vm.load(ticket_sale_proxy, ERC1967_ADMIN_SLOT))));
    }
}
