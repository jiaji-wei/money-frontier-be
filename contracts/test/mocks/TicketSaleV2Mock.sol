// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.27;

import {TicketSale} from "../../src/TicketSale.sol";

contract TicketSaleV2Mock is TicketSale {
    function version() external pure returns (uint256) {
        return 2;
    }
}
