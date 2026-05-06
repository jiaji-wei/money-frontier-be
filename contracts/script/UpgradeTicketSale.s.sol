// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.27;

import {console2} from "forge-std/console2.sol";
import {Script} from "forge-std/Script.sol";
import {TicketSale} from "../src/TicketSale.sol";
import {UnsafeUpgrades} from "openzeppelin-foundry-upgrades/Upgrades.sol";

contract UpgradeTicketSaleScript is Script {
    bytes32 internal constant ERC1967_ADMIN_SLOT = bytes32(uint256(keccak256("eip1967.proxy.admin")) - 1);
    bytes32 internal constant ERC1967_IMPLEMENTATION_SLOT =
        bytes32(uint256(keccak256("eip1967.proxy.implementation")) - 1);

    function run() public returns (address upgraded_implementation, address proxy_admin) {
        address proxy = vm.envAddress("TICKET_SALE_PROXY");
        address configured_proxy_admin = vm.envOr("PROXY_ADMIN", address(0));
        address new_implementation = vm.envOr("NEW_IMPLEMENTATION", address(0));
        address purchase_signer = vm.envOr("PURCHASE_SIGNER", address(0));
        uint256 broadcaster_private_key = vm.envOr("PRIVATE_KEY", uint256(0));

        proxy_admin = _proxyAdminAddress(proxy);
        if (configured_proxy_admin != address(0) && configured_proxy_admin != proxy_admin) {
            revert("PROXY_ADMIN mismatches proxy admin slot");
        }

        if (broadcaster_private_key == 0) {
            vm.startBroadcast();
        } else {
            vm.startBroadcast(broadcaster_private_key);
        }

        if (new_implementation == address(0)) {
            TicketSale implementation = new TicketSale();
            upgraded_implementation = address(implementation);
        } else {
            upgraded_implementation = new_implementation;
        }

        UnsafeUpgrades.upgradeProxy(proxy, upgraded_implementation, bytes(""));
        if (purchase_signer != address(0)) {
            TicketSale(proxy).setPurchaseSigner(purchase_signer);
        }

        vm.stopBroadcast();

        address active_implementation = _implementationAddress(proxy);
        console2.log("ticket_sale_proxy", proxy);
        console2.log("proxy_admin", proxy_admin);
        console2.log("upgraded_implementation", upgraded_implementation);
        console2.log("active_implementation", active_implementation);
        console2.log("purchase_signer", purchase_signer);
        require(active_implementation == upgraded_implementation, "implementation mismatch after upgrade");

        string memory output_file = vm.envOr("UPGRADE_OUTPUT_FILE", string(""));
        if (bytes(output_file).length > 0) {
            string memory json_key = "upgrade";
            string memory json = vm.serializeAddress(json_key, "proxy", proxy);
            json = vm.serializeAddress(json_key, "proxy_admin", proxy_admin);
            json = vm.serializeAddress(json_key, "upgraded_implementation", upgraded_implementation);
            json = vm.serializeAddress(json_key, "active_implementation", active_implementation);
            json = vm.serializeAddress(json_key, "purchase_signer", purchase_signer);
            vm.writeJson(json, output_file);
            console2.log("upgrade_output_file", output_file);
        }
    }

    function _proxyAdminAddress(address proxy) internal view returns (address) {
        return address(uint160(uint256(vm.load(proxy, ERC1967_ADMIN_SLOT))));
    }

    function _implementationAddress(address proxy) internal view returns (address) {
        return address(uint160(uint256(vm.load(proxy, ERC1967_IMPLEMENTATION_SLOT))));
    }
}
