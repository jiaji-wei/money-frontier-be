// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.27;

import {console2} from "forge-std/console2.sol";
import {Script} from "forge-std/Script.sol";
import {TicketSale} from "../src/TicketSale.sol";
import {TicketSaleProxy} from "../src/TicketSaleProxy.sol";

contract TicketSaleScript is Script {
    bytes32 internal constant ERC1967_ADMIN_SLOT = bytes32(uint256(keccak256("eip1967.proxy.admin")) - 1);

    function run() public returns (TicketSale sale) {
        address admin = vm.envAddress("OWNER");
        address pauser = vm.envOr("PAUSER", admin);
        address proxy_admin_owner = vm.envOr("PROXY_ADMIN_OWNER", admin);
        address treasury = vm.envAddress("TREASURY");
        address usdt = vm.envAddress("USDT_TOKEN");
        address usdc = vm.envAddress("USDC_TOKEN");

        address[] memory payment_tokens = new address[](2);
        payment_tokens[0] = usdt;
        payment_tokens[1] = usdc;

        vm.startBroadcast();

        TicketSale implementation = new TicketSale();
        bytes memory init_data = abi.encodeCall(TicketSale.initialize, (admin, pauser, treasury, payment_tokens));
        TicketSaleProxy proxy = new TicketSaleProxy(address(implementation), proxy_admin_owner, init_data);
        sale = TicketSale(address(proxy));

        vm.stopBroadcast();

        address proxy_admin = address(uint160(uint256(vm.load(address(proxy), ERC1967_ADMIN_SLOT))));
        console2.log("ticket_sale_implementation", address(implementation));
        console2.log("ticket_sale_proxy", address(proxy));
        console2.log("ticket_sale_proxy_admin", proxy_admin);
        console2.log("proxy_admin_owner", proxy_admin_owner);
        console2.log("default_admin", admin);
        console2.log("pauser", pauser);
        console2.log("treasury", treasury);
        console2.log("usdt_token", usdt);
        console2.log("usdc_token", usdc);

        string memory output_file = vm.envOr("DEPLOY_OUTPUT_FILE", string(""));
        if (bytes(output_file).length > 0) {
            string memory json = "deploy";
            json = vm.serializeAddress(json, "implementation", address(implementation));
            json = vm.serializeAddress(json, "proxy", address(proxy));
            json = vm.serializeAddress(json, "proxy_admin", proxy_admin);
            json = vm.serializeAddress(json, "proxy_admin_owner", proxy_admin_owner);
            json = vm.serializeAddress(json, "default_admin", admin);
            json = vm.serializeAddress(json, "pauser", pauser);
            json = vm.serializeAddress(json, "treasury", treasury);
            json = vm.serializeAddress(json, "usdt_token", usdt);
            json = vm.serializeAddress(json, "usdc_token", usdc);
            vm.writeJson(json, output_file);
            console2.log("deploy_output_file", output_file);
        }
    }
}
