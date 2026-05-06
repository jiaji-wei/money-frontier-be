// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.27;

import {console2} from "forge-std/console2.sol";
import {Script} from "forge-std/Script.sol";
import {TicketSale} from "../src/TicketSale.sol";
import {UnsafeUpgrades} from "openzeppelin-foundry-upgrades/Upgrades.sol";

contract TicketSaleScript is Script {
    bytes32 internal constant ERC1967_ADMIN_SLOT = bytes32(uint256(keccak256("eip1967.proxy.admin")) - 1);

    function run() public returns (TicketSale sale) {
        address admin = vm.envAddress("OWNER");
        address pauser = vm.envOr("PAUSER", admin);
        address proxy_admin_owner = vm.envOr("PROXY_ADMIN_OWNER", admin);
        address treasury = vm.envAddress("TREASURY");
        address usdt = vm.envAddress("USDT_TOKEN");
        address usdc = vm.envAddress("USDC_TOKEN");
        address purchase_signer = vm.envOr("PURCHASE_SIGNER", address(0));

        address[] memory payment_tokens = new address[](2);
        payment_tokens[0] = usdt;
        payment_tokens[1] = usdc;

        vm.startBroadcast();

        TicketSale implementation = new TicketSale();
        bytes memory init_data = abi.encodeCall(TicketSale.initialize, (admin, pauser, treasury, payment_tokens));
        address proxy = UnsafeUpgrades.deployTransparentProxy(address(implementation), proxy_admin_owner, init_data);
        sale = TicketSale(proxy);
        if (purchase_signer != address(0)) {
            sale.setPurchaseSigner(purchase_signer);
        }

        vm.stopBroadcast();

        address proxy_admin = address(uint160(uint256(vm.load(proxy, ERC1967_ADMIN_SLOT))));
        console2.log("ticket_sale_implementation", address(implementation));
        console2.log("ticket_sale_proxy", proxy);
        console2.log("ticket_sale_proxy_admin", proxy_admin);
        console2.log("proxy_admin_owner", proxy_admin_owner);
        console2.log("default_admin", admin);
        console2.log("pauser", pauser);
        console2.log("treasury", treasury);
        console2.log("purchase_signer", purchase_signer);
        console2.log("usdt_token", usdt);
        console2.log("usdc_token", usdc);

        string memory output_file = vm.envOr("DEPLOY_OUTPUT_FILE", string(""));
        if (bytes(output_file).length > 0) {
            _writeOutput(
                output_file,
                address(implementation),
                proxy,
                proxy_admin
            );
            console2.log("deploy_output_file", output_file);
        }
    }

    function _writeOutput(
        string memory output_file,
        address implementation,
        address proxy,
        address proxy_admin
    ) internal {
        address admin = vm.envAddress("OWNER");
        address pauser = vm.envOr("PAUSER", admin);
        address proxy_admin_owner = vm.envOr("PROXY_ADMIN_OWNER", admin);
        address treasury = vm.envAddress("TREASURY");
        address purchase_signer = vm.envOr("PURCHASE_SIGNER", address(0));
        address usdt = vm.envAddress("USDT_TOKEN");
        address usdc = vm.envAddress("USDC_TOKEN");

        string memory json_key = "deploy";
        string memory json = vm.serializeAddress(json_key, "implementation", implementation);
        json = vm.serializeAddress(json_key, "proxy", proxy);
        json = vm.serializeAddress(json_key, "proxy_admin", proxy_admin);
        json = vm.serializeAddress(json_key, "proxy_admin_owner", proxy_admin_owner);
        json = vm.serializeAddress(json_key, "default_admin", admin);
        json = vm.serializeAddress(json_key, "pauser", pauser);
        json = vm.serializeAddress(json_key, "treasury", treasury);
        json = vm.serializeAddress(json_key, "purchase_signer", purchase_signer);
        json = vm.serializeAddress(json_key, "usdt_token", usdt);
        json = vm.serializeAddress(json_key, "usdc_token", usdc);
        vm.writeJson(json, output_file);
    }
}
