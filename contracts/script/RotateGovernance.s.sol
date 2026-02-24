// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.27;

import {console2} from "forge-std/console2.sol";
import {Script} from "forge-std/Script.sol";
import {ProxyAdmin} from "openzeppelin-contracts/contracts/proxy/transparent/ProxyAdmin.sol";
import {TicketSale} from "../src/TicketSale.sol";

contract RotateGovernanceScript is Script {
    bytes32 internal constant DEFAULT_ADMIN_ROLE = 0x00;
    bytes32 internal constant PAUSER_ROLE = keccak256("PAUSER_ROLE");

    function run() external {
        address proxy = vm.envAddress("TICKET_SALE_PROXY");
        address proxy_admin = vm.envAddress("PROXY_ADMIN");
        address new_default_admin = vm.envAddress("NEW_DEFAULT_ADMIN");
        address new_pauser = vm.envAddress("NEW_PAUSER");

        address new_proxy_admin_owner = vm.envOr("NEW_PROXY_ADMIN_OWNER", address(0));
        address old_default_admin = vm.envOr("OLD_DEFAULT_ADMIN", address(0));
        address old_pauser = vm.envOr("OLD_PAUSER", address(0));

        TicketSale sale = TicketSale(proxy);
        ProxyAdmin admin = ProxyAdmin(proxy_admin);

        vm.startBroadcast();

        if (!sale.hasRole(DEFAULT_ADMIN_ROLE, new_default_admin)) {
            sale.grantRole(DEFAULT_ADMIN_ROLE, new_default_admin);
        }
        if (!sale.hasRole(PAUSER_ROLE, new_pauser)) {
            sale.grantRole(PAUSER_ROLE, new_pauser);
        }

        if (old_pauser != address(0) && old_pauser != new_pauser && sale.hasRole(PAUSER_ROLE, old_pauser)) {
            sale.revokeRole(PAUSER_ROLE, old_pauser);
        }
        if (
            old_default_admin != address(0) && old_default_admin != new_default_admin
                && sale.hasRole(DEFAULT_ADMIN_ROLE, old_default_admin)
        ) {
            sale.revokeRole(DEFAULT_ADMIN_ROLE, old_default_admin);
        }

        if (new_proxy_admin_owner != address(0) && admin.owner() != new_proxy_admin_owner) {
            admin.transferOwnership(new_proxy_admin_owner);
        }

        vm.stopBroadcast();

        console2.log("ticket_sale_proxy", proxy);
        console2.log("proxy_admin", proxy_admin);
        console2.log("proxy_admin_owner", admin.owner());
        console2.log("default_admin_enabled", new_default_admin, sale.hasRole(DEFAULT_ADMIN_ROLE, new_default_admin));
        console2.log("pauser_enabled", new_pauser, sale.hasRole(PAUSER_ROLE, new_pauser));
        if (old_default_admin != address(0)) {
            console2.log(
                "old_default_admin_enabled", old_default_admin, sale.hasRole(DEFAULT_ADMIN_ROLE, old_default_admin)
            );
        }
        if (old_pauser != address(0)) {
            console2.log("old_pauser_enabled", old_pauser, sale.hasRole(PAUSER_ROLE, old_pauser));
        }

        string memory output_file = vm.envOr("ROTATE_OUTPUT_FILE", string(""));
        if (bytes(output_file).length > 0) {
            string memory json = "rotation";
            json = vm.serializeAddress(json, "proxy", proxy);
            json = vm.serializeAddress(json, "proxy_admin", proxy_admin);
            json = vm.serializeAddress(json, "proxy_admin_owner", admin.owner());
            json = vm.serializeAddress(json, "new_default_admin", new_default_admin);
            json = vm.serializeAddress(json, "new_pauser", new_pauser);
            vm.writeJson(json, output_file);
            console2.log("rotate_output_file", output_file);
        }
    }
}
