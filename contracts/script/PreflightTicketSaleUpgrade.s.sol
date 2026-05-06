// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.27;

import {console2} from "forge-std/console2.sol";
import {Script} from "forge-std/Script.sol";
import {ProxyAdmin} from "openzeppelin-contracts/contracts/proxy/transparent/ProxyAdmin.sol";
import {TicketSale} from "../src/TicketSale.sol";

contract PreflightTicketSaleUpgradeScript is Script {
    bytes32 internal constant ERC1967_ADMIN_SLOT = bytes32(uint256(keccak256("eip1967.proxy.admin")) - 1);
    bytes32 internal constant ERC1967_IMPLEMENTATION_SLOT =
        bytes32(uint256(keccak256("eip1967.proxy.implementation")) - 1);
    bytes4 internal constant PURCHASE_SIGNER_SELECTOR = bytes4(keccak256("purchase_signer()"));

    function run() external view {
        address proxy = vm.envAddress("TICKET_SALE_PROXY");

        address actual_proxy_admin = _slotAddress(proxy, ERC1967_ADMIN_SLOT);
        address actual_implementation = _slotAddress(proxy, ERC1967_IMPLEMENTATION_SLOT);
        address actual_proxy_admin_owner = ProxyAdmin(actual_proxy_admin).owner();
        (bool purchase_signer_supported, address actual_purchase_signer) = _purchaseSigner(proxy);

        address expected_proxy_admin = vm.envOr("EXPECTED_PROXY_ADMIN", address(0));
        address expected_proxy_admin_owner = vm.envOr("EXPECTED_PROXY_ADMIN_OWNER", address(0));
        address expected_implementation = vm.envOr("EXPECTED_IMPLEMENTATION", address(0));
        address expected_purchase_signer = vm.envOr("EXPECTED_PURCHASE_SIGNER", address(0));

        if (expected_proxy_admin != address(0)) {
            require(actual_proxy_admin == expected_proxy_admin, "proxy admin mismatch");
        }
        if (expected_proxy_admin_owner != address(0)) {
            require(actual_proxy_admin_owner == expected_proxy_admin_owner, "proxy admin owner mismatch");
        }
        if (expected_implementation != address(0)) {
            require(actual_implementation == expected_implementation, "implementation mismatch");
        }
        if (expected_purchase_signer != address(0)) {
            require(purchase_signer_supported, "purchase signer unavailable on current implementation");
            require(actual_purchase_signer == expected_purchase_signer, "purchase signer mismatch");
        }

        console2.log("ticket_sale_proxy", proxy);
        console2.log("proxy_admin", actual_proxy_admin);
        console2.log("proxy_admin_owner", actual_proxy_admin_owner);
        console2.log("implementation", actual_implementation);
        console2.log("purchase_signer_supported", purchase_signer_supported);
        console2.log("purchase_signer", actual_purchase_signer);
    }

    function _slotAddress(address target, bytes32 slot) internal view returns (address) {
        return address(uint160(uint256(vm.load(target, slot))));
    }

    function _purchaseSigner(address proxy) internal view returns (bool supported, address signer) {
        (bool success, bytes memory result) = proxy.staticcall(abi.encodeWithSelector(PURCHASE_SIGNER_SELECTOR));
        if (!success || result.length < 32) {
            return (false, address(0));
        }

        return (true, abi.decode(result, (address)));
    }
}
