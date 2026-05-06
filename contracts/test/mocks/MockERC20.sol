// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import {ERC20} from "openzeppelin-contracts/contracts/token/ERC20/ERC20.sol";
import {Ownable} from "openzeppelin-contracts/contracts/access/Ownable.sol";

contract MockERC20 is ERC20, Ownable {
    uint8 private immutable _token_decimals;

    constructor(string memory token_name, string memory token_symbol, uint8 token_decimals)
        ERC20(token_name, token_symbol)
        Ownable(msg.sender)
    {
        _token_decimals = token_decimals;
    }

    function decimals() public view override returns (uint8) {
        return _token_decimals;
    }

    function mint(address to, uint256 amount) external onlyOwner {
        _mint(to, amount);
    }
}
