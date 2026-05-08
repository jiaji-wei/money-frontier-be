// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.27;

import {console2} from "forge-std/console2.sol";
import {Script} from "forge-std/Script.sol";
import {TicketSale} from "../src/TicketSale.sol";
import {UnsafeUpgrades} from "openzeppelin-foundry-upgrades/Upgrades.sol";

contract TicketSaleScript is Script {
    bytes32 internal constant ERC1967_ADMIN_SLOT = bytes32(uint256(keccak256("eip1967.proxy.admin")) - 1);
    bytes32 internal constant DEFAULT_ADMIN_ROLE = 0x00;

    struct DeployConfig {
        address admin;
        address final_admin;
        address pauser;
        address proxy_admin_owner;
        address treasury;
        address usdt;
        address usdc;
        address purchase_signer;
    }

    function run() public returns (TicketSale sale) {
        DeployConfig memory config = _readConfig();

        address[] memory payment_tokens = new address[](2);
        payment_tokens[0] = config.usdt;
        payment_tokens[1] = config.usdc;

        vm.startBroadcast();

        TicketSale implementation = new TicketSale();
        bytes memory init_data =
            abi.encodeCall(TicketSale.initialize, (config.admin, config.pauser, config.treasury, payment_tokens));
        address proxy = UnsafeUpgrades.deployTransparentProxy(address(implementation), config.proxy_admin_owner, init_data);
        sale = TicketSale(proxy);
        if (config.purchase_signer != address(0)) {
            sale.setPurchaseSigner(config.purchase_signer);
        }
        _configurePriceSchedules(sale);
        _handoverDefaultAdmin(sale, config.admin, config.final_admin);

        vm.stopBroadcast();

        address proxy_admin = address(uint160(uint256(vm.load(proxy, ERC1967_ADMIN_SLOT))));
        console2.log("ticket_sale_implementation", address(implementation));
        console2.log("ticket_sale_proxy", proxy);
        console2.log("ticket_sale_proxy_admin", proxy_admin);
        console2.log("proxy_admin_owner", config.proxy_admin_owner);
        console2.log("default_admin", config.admin);
        console2.log("final_default_admin", config.final_admin);
        console2.log("pauser", config.pauser);
        console2.log("treasury", config.treasury);
        console2.log("purchase_signer", config.purchase_signer);
        console2.log("usdt_token", config.usdt);
        console2.log("usdc_token", config.usdc);

        string memory output_file = vm.envOr("DEPLOY_OUTPUT_FILE", string(""));
        if (bytes(output_file).length > 0) {
            _writeOutput(output_file, address(implementation), proxy, proxy_admin, config);
            console2.log("deploy_output_file", output_file);
        }
    }

    function _readConfig() internal view returns (DeployConfig memory config) {
        config.admin = vm.envAddress("OWNER");
        config.final_admin = vm.envOr("FINAL_OWNER", config.admin);
        config.pauser = vm.envOr("PAUSER", config.admin);
        config.proxy_admin_owner = vm.envOr("PROXY_ADMIN_OWNER", config.admin);
        config.treasury = vm.envAddress("TREASURY");
        config.usdt = vm.envAddress("USDT_TOKEN");
        config.usdc = vm.envAddress("USDC_TOKEN");
        config.purchase_signer = vm.envOr("PURCHASE_SIGNER", address(0));
        require(config.final_admin != address(0), "FINAL_OWNER cannot be zero");
    }

    function _configurePriceSchedules(TicketSale sale) internal {
        _configurePriceSchedule(sale, 1, "LEVEL_1_START_TIMESTAMPS", "LEVEL_1_PRICES");
        _configurePriceSchedule(sale, 2, "LEVEL_2_START_TIMESTAMPS", "LEVEL_2_PRICES");
        _configurePriceSchedule(sale, 3, "LEVEL_3_START_TIMESTAMPS", "LEVEL_3_PRICES");
    }

    function _configurePriceSchedule(
        TicketSale sale,
        uint8 level_id,
        string memory starts_env,
        string memory prices_env
    ) internal {
        string memory starts_raw = vm.envOr(starts_env, string(""));
        if (bytes(starts_raw).length == 0) {
            return;
        }

        string memory prices_raw = vm.envOr(prices_env, string(""));
        require(bytes(prices_raw).length > 0, "price schedule prices missing");

        uint256[] memory start_values = vm.envUint(starts_env, ",");
        uint256[] memory price_values = vm.envUint(prices_env, ",");
        require(start_values.length == price_values.length, "price schedule length mismatch");
        require(start_values.length > 0, "price schedule empty");

        uint64[] memory starts = new uint64[](start_values.length);
        for (uint256 i = 0; i < start_values.length; i++) {
            require(start_values[i] <= type(uint64).max, "price schedule timestamp too large");
            starts[i] = uint64(start_values[i]);
        }

        sale.setPriceSchedule(level_id, starts, price_values);
    }

    function _handoverDefaultAdmin(
        TicketSale sale,
        address temporary_admin,
        address final_admin
    ) internal {
        if (final_admin == temporary_admin) {
            return;
        }

        if (!sale.hasRole(DEFAULT_ADMIN_ROLE, final_admin)) {
            sale.grantRole(DEFAULT_ADMIN_ROLE, final_admin);
        }
        if (sale.hasRole(DEFAULT_ADMIN_ROLE, temporary_admin)) {
            sale.revokeRole(DEFAULT_ADMIN_ROLE, temporary_admin);
        }
    }

    function _writeOutput(
        string memory output_file,
        address implementation,
        address proxy,
        address proxy_admin,
        DeployConfig memory config
    ) internal {
        string memory json_key = "deploy";
        string memory json = vm.serializeAddress(json_key, "implementation", implementation);
        json = vm.serializeAddress(json_key, "proxy", proxy);
        json = vm.serializeAddress(json_key, "proxy_admin", proxy_admin);
        json = vm.serializeAddress(json_key, "proxy_admin_owner", config.proxy_admin_owner);
        json = vm.serializeAddress(json_key, "temporary_default_admin", config.admin);
        json = vm.serializeAddress(json_key, "default_admin", config.final_admin);
        json = vm.serializeAddress(json_key, "pauser", config.pauser);
        json = vm.serializeAddress(json_key, "treasury", config.treasury);
        json = vm.serializeAddress(json_key, "purchase_signer", config.purchase_signer);
        json = vm.serializeAddress(json_key, "usdt_token", config.usdt);
        json = vm.serializeAddress(json_key, "usdc_token", config.usdc);
        vm.writeJson(json, output_file);
    }
}
