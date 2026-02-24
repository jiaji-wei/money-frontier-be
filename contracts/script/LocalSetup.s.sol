// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.27;

import {console2} from "forge-std/console2.sol";
import {Script} from "forge-std/Script.sol";
import {MockERC20} from "../test/mocks/MockERC20.sol";
import {TicketSale} from "../src/TicketSale.sol";
import {TicketSaleProxy} from "../src/TicketSaleProxy.sol";

contract LocalSetupScript is Script {
    bytes32 internal constant ERC1967_ADMIN_SLOT = bytes32(uint256(keccak256("eip1967.proxy.admin")) - 1);

    function run() public {
        address owner = vm.envAddress("OWNER");
        address pauser = vm.envOr("PAUSER", owner);
        address proxy_admin_owner = vm.envOr("PROXY_ADMIN_OWNER", owner);
        address treasury = vm.envOr("TREASURY", owner);
        address buyer = vm.envOr("BUYER", address(0));

        vm.startBroadcast();

        (address usdt, address usdc, address implementation, address proxy) =
            _deployContracts(owner, pauser, proxy_admin_owner, treasury);
        _configureDefaultSchedules(TicketSale(proxy));
        _seedBuyer(MockERC20(usdt), MockERC20(usdc), buyer);

        vm.stopBroadcast();

        address proxy_admin = address(uint160(uint256(vm.load(proxy, ERC1967_ADMIN_SLOT))));
        console2.log("local_usdt", usdt);
        console2.log("local_usdc", usdc);
        console2.log("ticket_sale_implementation", implementation);
        console2.log("ticket_sale_proxy", proxy);
        console2.log("ticket_sale_proxy_admin", proxy_admin);
        console2.log("proxy_admin_owner", proxy_admin_owner);
        console2.log("default_admin", owner);
        console2.log("pauser", pauser);
        console2.log("treasury", treasury);
        console2.log("buyer_seeded", buyer);
    }

    function _deployContracts(address owner, address pauser, address proxy_admin_owner, address treasury)
        internal
        returns (address usdt, address usdc, address implementation, address proxy)
    {
        MockERC20 usdt_contract = new MockERC20("Tether USD", "USDT", 6);
        MockERC20 usdc_contract = new MockERC20("USD Coin", "USDC", 6);
        usdt = address(usdt_contract);
        usdc = address(usdc_contract);

        address[] memory payment_tokens = new address[](2);
        payment_tokens[0] = usdt;
        payment_tokens[1] = usdc;

        TicketSale implementation_contract = new TicketSale();
        implementation = address(implementation_contract);

        bytes memory init_data = abi.encodeCall(TicketSale.initialize, (owner, pauser, treasury, payment_tokens));
        proxy = address(new TicketSaleProxy(implementation, proxy_admin_owner, init_data));
    }

    function _configureDefaultSchedules(TicketSale sale) internal {
        uint64[] memory level_1_times = new uint64[](2);
        uint256[] memory level_1_prices = new uint256[](2);
        level_1_times[0] = 0;
        level_1_times[1] = 2_000_000_000;
        level_1_prices[0] = 100e6;
        level_1_prices[1] = 80e6;
        sale.setPriceSchedule(1, level_1_times, level_1_prices);

        uint64[] memory level_2_times = new uint64[](2);
        uint256[] memory level_2_prices = new uint256[](2);
        level_2_times[0] = 0;
        level_2_times[1] = 2_000_000_000;
        level_2_prices[0] = 200e6;
        level_2_prices[1] = 150e6;
        sale.setPriceSchedule(2, level_2_times, level_2_prices);

        uint64[] memory level_3_times = new uint64[](2);
        uint256[] memory level_3_prices = new uint256[](2);
        level_3_times[0] = 0;
        level_3_times[1] = 2_000_000_000;
        level_3_prices[0] = 300e6;
        level_3_prices[1] = 250e6;
        sale.setPriceSchedule(3, level_3_times, level_3_prices);
    }

    function _seedBuyer(MockERC20 usdt, MockERC20 usdc, address buyer) internal {
        if (buyer == address(0)) {
            return;
        }

        usdt.mint(buyer, 1_000_000e6);
        usdc.mint(buyer, 1_000_000e6);
    }
}
