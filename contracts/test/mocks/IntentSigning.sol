// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.27;

import {MessageHashUtils} from "openzeppelin-contracts/contracts/utils/cryptography/MessageHashUtils.sol";

library IntentSigning {
    function digest(
        address sale_contract,
        uint256 chain_id,
        address buyer,
        address payment_token,
        uint8[] memory level_ids,
        uint256[] memory quantities,
        bytes32 intent_id,
        uint256 final_total_amount,
        uint64 expires_at
    ) internal pure returns (bytes32) {
        uint256[] memory level_ids_u256 = new uint256[](level_ids.length);
        for (uint256 i = 0; i < level_ids.length; i++) {
            level_ids_u256[i] = uint256(level_ids[i]);
        }

        return keccak256(
            abi.encode(
                sale_contract,
                chain_id,
                buyer,
                payment_token,
                level_ids_u256,
                quantities,
                intent_id,
                final_total_amount,
                uint256(expires_at)
            )
        );
    }

    function ethSignedDigest(
        bytes32 digest_value
    ) internal pure returns (bytes32) {
        return MessageHashUtils.toEthSignedMessageHash(digest_value);
    }
}
