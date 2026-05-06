// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.27;

import {console2} from "forge-std/console2.sol";
import {Script} from "forge-std/Script.sol";
import {TicketSale} from "../src/TicketSale.sol";

contract VerifyTicketSaleUpgradeScript is Script {
    bytes32 internal constant DEFAULT_ADMIN_ROLE = 0x00;
    bytes32 internal constant PAUSER_ROLE = keccak256("PAUSER_ROLE");
    bytes32 internal constant ERC1967_ADMIN_SLOT = bytes32(uint256(keccak256("eip1967.proxy.admin")) - 1);
    bytes32 internal constant ERC1967_IMPLEMENTATION_SLOT =
        bytes32(uint256(keccak256("eip1967.proxy.implementation")) - 1);

    function run() external view {
        address proxy = vm.envAddress("TICKET_SALE_PROXY");
        address expected_proxy_admin = vm.envAddress("PROXY_ADMIN");
        address expected_implementation = vm.envAddress("EXPECTED_IMPLEMENTATION");
        address expected_admin = vm.envAddress("EXPECTED_DEFAULT_ADMIN");
        address expected_pauser = vm.envAddress("EXPECTED_PAUSER");
        address expected_treasury = vm.envAddress("EXPECTED_TREASURY");
        address expected_purchase_signer = vm.envAddress("EXPECTED_PURCHASE_SIGNER");

        address actual_proxy_admin = _slotAddress(proxy, ERC1967_ADMIN_SLOT);
        address actual_implementation = _slotAddress(proxy, ERC1967_IMPLEMENTATION_SLOT);

        require(actual_proxy_admin == expected_proxy_admin, "proxy admin mismatch");
        require(actual_implementation == expected_implementation, "implementation mismatch");

        TicketSale sale = TicketSale(proxy);
        require(sale.hasRole(DEFAULT_ADMIN_ROLE, expected_admin), "default admin role missing");
        require(sale.hasRole(PAUSER_ROLE, expected_pauser), "pauser role missing");
        require(sale.treasury() == expected_treasury, "treasury mismatch");
        require(sale.purchase_signer() == expected_purchase_signer, "purchase signer mismatch");
        require(sale.next_order_id() >= 1, "next_order_id invalid");

        console2.log("ticket_sale_proxy", proxy);
        console2.log("proxy_admin", actual_proxy_admin);
        console2.log("implementation", actual_implementation);
        console2.log("default_admin_verified", expected_admin);
        console2.log("pauser_verified", expected_pauser);
        console2.log("treasury_verified", expected_treasury);
        console2.log("purchase_signer_verified", expected_purchase_signer);
        console2.log("next_order_id", sale.next_order_id());
    }

    function _slotAddress(address target, bytes32 slot) internal view returns (address) {
        return address(uint160(uint256(vm.load(target, slot))));
    }
}
