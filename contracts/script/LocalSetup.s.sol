// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.27;

import {console2} from "forge-std/console2.sol";
import {Script} from "forge-std/Script.sol";
import {MockERC20} from "../test/mocks/MockERC20.sol";
import {TicketSale} from "../src/TicketSale.sol";
import {UnsafeUpgrades} from "openzeppelin-foundry-upgrades/Upgrades.sol";

contract LocalSetupScript is Script {
    bytes32 internal constant ERC1967_ADMIN_SLOT = bytes32(uint256(keccak256("eip1967.proxy.admin")) - 1);
    bytes32 internal constant DEFAULT_ADMIN_ROLE = 0x00;

    function run() public {
        address target_owner = vm.envAddress("OWNER");
        address target_pauser = vm.envOr("PAUSER", target_owner);
        address target_proxy_admin_owner = vm.envOr("PROXY_ADMIN_OWNER", target_owner);
        address treasury = vm.envOr("TREASURY", target_owner);
        address buyer = vm.envOr("BUYER", address(0));
        address purchase_signer = vm.envOr("PURCHASE_SIGNER", address(0));

        vm.startBroadcast();
        (, address broadcaster,) = vm.readCallers();

        (address usdt, address usdc, address implementation, address proxy) =
            _deployContracts(broadcaster, target_pauser, target_proxy_admin_owner, treasury);

        TicketSale sale = TicketSale(proxy);
        _configureDefaultSchedules(sale);
        _configurePurchaseSigner(sale, purchase_signer);
        _seedBuyer(MockERC20(usdt), MockERC20(usdc), buyer);
        _handoverDefaultAdmin(sale, broadcaster, target_owner);

        vm.stopBroadcast();

        address proxy_admin = address(uint160(uint256(vm.load(proxy, ERC1967_ADMIN_SLOT))));
        console2.log("local_usdt", usdt);
        console2.log("local_usdc", usdc);
        console2.log("ticket_sale_implementation", implementation);
        console2.log("ticket_sale_proxy", proxy);
        console2.log("ticket_sale_proxy_admin", proxy_admin);
        console2.log("broadcaster", broadcaster);
        console2.log("proxy_admin_owner", target_proxy_admin_owner);
        console2.log("default_admin", target_owner);
        console2.log("pauser", target_pauser);
        console2.log("treasury", treasury);
        console2.log("purchase_signer", purchase_signer);
        console2.log("buyer_seeded", buyer);

        _writeOutput(usdt, usdc, implementation, proxy, proxy_admin);
    }

    function _deployContracts(address owner, address pauser, address proxy_admin_owner, address treasury)
        internal
        returns (address usdt, address usdc, address implementation, address proxy)
    {
        MockERC20 usdt_contract = new MockERC20("Test Tether USD", "USDT", 18);
        MockERC20 usdc_contract = new MockERC20("Test USD Coin", "USDC", 18);
        usdt = address(usdt_contract);
        usdc = address(usdc_contract);

        address[] memory payment_tokens = new address[](2);
        payment_tokens[0] = usdt;
        payment_tokens[1] = usdc;

        TicketSale implementation_contract = new TicketSale();
        implementation = address(implementation_contract);

        bytes memory init_data = abi.encodeCall(TicketSale.initialize, (owner, pauser, treasury, payment_tokens));
        proxy = UnsafeUpgrades.deployTransparentProxy(implementation, proxy_admin_owner, init_data);
    }

    function _handoverDefaultAdmin(
        TicketSale sale,
        address broadcaster,
        address target_owner
    ) internal {
        if (!sale.hasRole(DEFAULT_ADMIN_ROLE, target_owner)) {
            sale.grantRole(DEFAULT_ADMIN_ROLE, target_owner);
        }
        if (broadcaster != target_owner && sale.hasRole(DEFAULT_ADMIN_ROLE, broadcaster)) {
            sale.revokeRole(DEFAULT_ADMIN_ROLE, broadcaster);
        }
    }

    function _configureDefaultSchedules(TicketSale sale) internal {
        uint64[] memory level_1_times = new uint64[](2);
        uint256[] memory level_1_prices = new uint256[](2);
        level_1_times[0] = 0;
        level_1_times[1] = 2_000_000_000;
        level_1_prices[0] = 100e18;
        level_1_prices[1] = 80e18;
        sale.setPriceSchedule(1, level_1_times, level_1_prices);

        uint64[] memory level_2_times = new uint64[](2);
        uint256[] memory level_2_prices = new uint256[](2);
        level_2_times[0] = 0;
        level_2_times[1] = 2_000_000_000;
        level_2_prices[0] = 200e18;
        level_2_prices[1] = 150e18;
        sale.setPriceSchedule(2, level_2_times, level_2_prices);

        uint64[] memory level_3_times = new uint64[](2);
        uint256[] memory level_3_prices = new uint256[](2);
        level_3_times[0] = 0;
        level_3_times[1] = 2_000_000_000;
        level_3_prices[0] = 300e18;
        level_3_prices[1] = 250e18;
        sale.setPriceSchedule(3, level_3_times, level_3_prices);
    }

    function _seedBuyer(MockERC20 usdt, MockERC20 usdc, address buyer) internal {
        if (buyer == address(0)) {
            return;
        }

        usdt.mint(buyer, 1_000_000e18);
        usdc.mint(buyer, 1_000_000e18);
    }

    function _configurePurchaseSigner(
        TicketSale sale,
        address purchase_signer
    ) internal {
        if (purchase_signer == address(0)) {
            return;
        }

        sale.setPurchaseSigner(purchase_signer);
    }

    function _writeOutput(
        address usdt,
        address usdc,
        address implementation,
        address proxy,
        address proxy_admin
    ) internal {
        string memory output_file = vm.envOr("DEPLOY_OUTPUT_FILE", string(""));
        if (bytes(output_file).length == 0) {
            return;
        }

        address target_owner = vm.envAddress("OWNER");
        address target_pauser = vm.envOr("PAUSER", target_owner);
        address target_proxy_admin_owner = vm.envOr("PROXY_ADMIN_OWNER", target_owner);
        address treasury = vm.envOr("TREASURY", target_owner);

        string memory json_key = "local_setup";
        string memory json = vm.serializeAddress(json_key, "usdt", usdt);
        json = vm.serializeAddress(json_key, "usdc", usdc);
        json = vm.serializeAddress(json_key, "implementation", implementation);
        json = vm.serializeAddress(json_key, "proxy", proxy);
        json = vm.serializeAddress(json_key, "proxy_admin", proxy_admin);
        json = vm.serializeAddress(json_key, "proxy_admin_owner", target_proxy_admin_owner);
        json = vm.serializeAddress(json_key, "default_admin", target_owner);
        json = vm.serializeAddress(json_key, "pauser", target_pauser);
        json = vm.serializeAddress(json_key, "treasury", treasury);
        json = vm.serializeAddress(
            json_key,
            "purchase_signer",
            vm.envOr("PURCHASE_SIGNER", address(0))
        );
        vm.writeJson(json, output_file);
        console2.log("deploy_output_file", output_file);
    }
}
