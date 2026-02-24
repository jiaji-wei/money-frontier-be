// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.27;

import {console2} from "forge-std/console2.sol";
import {Script} from "forge-std/Script.sol";
import {TicketSale} from "../src/TicketSale.sol";

contract EmergencyPauseScript is Script {
    function run() external {
        address proxy = vm.envAddress("TICKET_SALE_PROXY");
        TicketSale sale = TicketSale(proxy);

        vm.startBroadcast();
        sale.pause();
        vm.stopBroadcast();

        console2.log("ticket_sale_proxy", proxy);
        console2.log("paused", sale.paused());
        require(sale.paused(), "pause failed");
    }
}
