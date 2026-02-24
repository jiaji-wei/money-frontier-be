// SPDX-License-Identifier: MIT
pragma solidity ^0.8.27;

import {TransparentUpgradeableProxy} from "@openzeppelin/contracts/proxy/transparent/TransparentUpgradeableProxy.sol";

contract TicketSaleProxy is TransparentUpgradeableProxy {
    constructor(address implementation, address adminOwner, bytes memory initData)
        TransparentUpgradeableProxy(implementation, adminOwner, initData)
    {}
}
