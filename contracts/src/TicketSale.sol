// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.27;

import {AccessControlUpgradeable} from "openzeppelin-contracts-upgradeable/contracts/access/AccessControlUpgradeable.sol";
import {ECDSA} from "openzeppelin-contracts/contracts/utils/cryptography/ECDSA.sol";
import {MessageHashUtils} from "openzeppelin-contracts/contracts/utils/cryptography/MessageHashUtils.sol";
import {Initializable} from "openzeppelin-contracts-upgradeable/contracts/proxy/utils/Initializable.sol";
import {PausableUpgradeable} from "openzeppelin-contracts-upgradeable/contracts/utils/PausableUpgradeable.sol";
import {SafeERC20} from "openzeppelin-contracts/contracts/token/ERC20/utils/SafeERC20.sol";
import {IERC20} from "openzeppelin-contracts/contracts/token/ERC20/IERC20.sol";

contract TicketSale is
    Initializable,
    PausableUpgradeable,
    AccessControlUpgradeable
{
    using SafeERC20 for IERC20;

    uint8 public constant MIN_LEVEL = 1;
    uint8 public constant MAX_LEVEL = 3;

    bytes32 public constant PAUSER_ROLE = keccak256("PAUSER_ROLE");

    struct PriceSchedule {
        uint64[] start_timestamps;
        uint256[] prices;
    }

    struct PurchaseAuthorization {
        bytes32 intent_id;
        uint256 final_total_amount;
        uint64 expires_at;
        bytes signature;
    }

    error ZeroAddress();
    error InvalidLevel(uint8 level_id);
    error InvalidScheduleLength();
    error UnsortedTimestamp(uint256 index);
    error EmptyOrder();
    error MismatchedOrderInputLength();
    error ZeroQuantity(uint256 index);
    error UnsupportedPaymentToken(address token);
    error PriceScheduleMissing(uint8 level_id);
    error PriceNotStarted(uint8 level_id, uint64 now_timestamp);
    error ReentrancyBlocked();
    error PurchaseSignerMissing();
    error InvalidPurchaseSigner(address recovered_signer);
    error AuthorizationExpired(uint64 expires_at, uint64 now_timestamp);
    error IntentAlreadyConsumed(bytes32 intent_id);
    error FinalAmountExceedsQuotedTotal(
        uint256 final_total_amount,
        uint256 quoted_total_amount
    );

    event TreasuryUpdated(
        address indexed previous_treasury,
        address indexed new_treasury
    );
    event PurchaseSignerUpdated(
        address indexed previous_signer,
        address indexed new_signer
    );
    event PaymentTokenUpdated(address indexed token, bool enabled);
    event PriceScheduleUpdated(
        uint8 indexed level_id,
        uint64[] start_timestamps,
        uint256[] prices
    );
    event TicketsPurchased(
        uint256 indexed order_id,
        address indexed buyer,
        address indexed payment_token,
        uint256 total_amount,
        uint8[] level_ids,
        uint256[] quantities,
        uint256[] unit_prices,
        uint256 purchased_at
    );
    event TicketsPurchasedWithIntent(
        uint256 indexed order_id,
        bytes32 indexed intent_id,
        address indexed buyer,
        address payment_token,
        uint256 total_amount,
        uint8[] level_ids,
        uint256[] quantities,
        uint256[] unit_prices,
        uint256 purchased_at
    );

    mapping(uint8 => PriceSchedule) private _price_schedules;
    mapping(address => bool) public payment_tokens;

    address public treasury;
    uint256 public next_order_id;
    uint256 private _reentrancy_lock;
    address public purchase_signer;
    mapping(bytes32 => bool) public consumed_intents;

    /// @custom:oz-upgrades-unsafe-allow constructor
    constructor() {
        _disableInitializers();
    }

    function initialize(
        address default_admin,
        address pauser,
        address initial_treasury,
        address[] calldata initial_payment_tokens
    ) external initializer {
        if (
            default_admin == address(0) ||
            pauser == address(0) ||
            initial_treasury == address(0)
        ) {
            revert ZeroAddress();
        }

        __Pausable_init();
        __AccessControl_init();

        _grantRole(DEFAULT_ADMIN_ROLE, default_admin);
        _grantRole(PAUSER_ROLE, pauser);

        treasury = initial_treasury;
        next_order_id = 1;
        emit TreasuryUpdated(address(0), initial_treasury);

        for (uint256 i = 0; i < initial_payment_tokens.length; i++) {
            address token = initial_payment_tokens[i];
            if (token == address(0)) {
                revert ZeroAddress();
            }
            payment_tokens[token] = true;
            emit PaymentTokenUpdated(token, true);
        }
    }

    function setTreasury(
        address new_treasury
    ) external onlyRole(DEFAULT_ADMIN_ROLE) {
        if (new_treasury == address(0)) {
            revert ZeroAddress();
        }

        address previous_treasury = treasury;
        treasury = new_treasury;
        emit TreasuryUpdated(previous_treasury, new_treasury);
    }

    function setPaymentToken(
        address token,
        bool enabled
    ) external onlyRole(DEFAULT_ADMIN_ROLE) {
        if (token == address(0)) {
            revert ZeroAddress();
        }

        payment_tokens[token] = enabled;
        emit PaymentTokenUpdated(token, enabled);
    }

    function setPurchaseSigner(
        address new_purchase_signer
    ) external onlyRole(DEFAULT_ADMIN_ROLE) {
        address previous_signer = purchase_signer;
        purchase_signer = new_purchase_signer;
        emit PurchaseSignerUpdated(previous_signer, new_purchase_signer);
    }

    function pause() external onlyRole(PAUSER_ROLE) {
        _pause();
    }

    function unpause() external onlyRole(PAUSER_ROLE) {
        _unpause();
    }

    function setPriceSchedule(
        uint8 level_id,
        uint64[] calldata start_timestamps,
        uint256[] calldata prices
    ) external onlyRole(DEFAULT_ADMIN_ROLE) {
        _validateLevel(level_id);
        if (
            start_timestamps.length == 0 ||
            start_timestamps.length != prices.length
        ) {
            revert InvalidScheduleLength();
        }

        for (uint256 i = 1; i < start_timestamps.length; i++) {
            if (start_timestamps[i] <= start_timestamps[i - 1]) {
                revert UnsortedTimestamp(i);
            }
        }

        PriceSchedule storage schedule = _price_schedules[level_id];
        delete schedule.start_timestamps;
        delete schedule.prices;

        for (uint256 i = 0; i < start_timestamps.length; i++) {
            schedule.start_timestamps.push(start_timestamps[i]);
            schedule.prices.push(prices[i]);
        }

        emit PriceScheduleUpdated(level_id, start_timestamps, prices);
    }

    function getPriceSchedule(
        uint8 level_id
    )
        external
        view
        returns (uint64[] memory start_timestamps, uint256[] memory prices)
    {
        _validateLevel(level_id);
        PriceSchedule storage schedule = _price_schedules[level_id];
        return (schedule.start_timestamps, schedule.prices);
    }

    function currentPrice(uint8 level_id) external view returns (uint256) {
        return _resolveCurrentPrice(level_id, uint64(block.timestamp));
    }

    function quote(
        uint8[] calldata level_ids,
        uint256[] calldata quantities
    )
        external
        view
        returns (uint256 total_amount, uint256[] memory unit_prices)
    {
        _validateOrderInput(level_ids, quantities);

        uint64 now_timestamp = uint64(block.timestamp);
        unit_prices = new uint256[](level_ids.length);

        for (uint256 i = 0; i < level_ids.length; i++) {
            uint256 quantity = quantities[i];
            if (quantity == 0) {
                revert ZeroQuantity(i);
            }

            uint256 unit_price = _resolveCurrentPrice(
                level_ids[i],
                now_timestamp
            );
            unit_prices[i] = unit_price;
            total_amount += unit_price * quantity;
        }
    }

    function purchase(
        address payment_token,
        uint8[] calldata level_ids,
        uint256[] calldata quantities
    )
        external
        whenNotPaused
        nonReentrant
        returns (uint256 order_id, uint256 total_amount)
    {
        if (!payment_tokens[payment_token]) {
            revert UnsupportedPaymentToken(payment_token);
        }

        uint256[] memory unit_prices;
        (total_amount, unit_prices) = _quoteCurrentOrder(level_ids, quantities);

        IERC20(payment_token).safeTransferFrom(
            msg.sender,
            treasury,
            total_amount
        );

        order_id = next_order_id;
        next_order_id = order_id + 1;

        emit TicketsPurchased(
            order_id,
            msg.sender,
            payment_token,
            total_amount,
            level_ids,
            quantities,
            unit_prices,
            block.timestamp
        );
    }

    function purchaseWithAuthorization(
        address payment_token,
        uint8[] calldata level_ids,
        uint256[] calldata quantities,
        bytes32 intent_id,
        uint256 final_total_amount,
        uint64 expires_at,
        bytes calldata signature
    )
        external
        whenNotPaused
        nonReentrant
        returns (uint256 order_id, uint256 total_amount)
    {
        PurchaseAuthorization memory authorization = PurchaseAuthorization({
            intent_id: intent_id,
            final_total_amount: final_total_amount,
            expires_at: expires_at,
            signature: signature
        });

        return
            _purchaseWithAuthorization(
                payment_token,
                level_ids,
                quantities,
                authorization
            );
    }

    function _purchaseWithAuthorization(
        address payment_token,
        uint8[] calldata level_ids,
        uint256[] calldata quantities,
        PurchaseAuthorization memory authorization
    ) internal returns (uint256 order_id, uint256 total_amount) {
        if (!payment_tokens[payment_token]) {
            revert UnsupportedPaymentToken(payment_token);
        }

        uint256 quoted_total_amount;
        uint256[] memory unit_prices;
        (quoted_total_amount, unit_prices) = _quoteCurrentOrder(
            level_ids,
            quantities
        );

        _verifyAndConsumeIntent(
            payment_token,
            level_ids,
            quantities,
            authorization,
            quoted_total_amount
        );

        IERC20(payment_token).safeTransferFrom(
            msg.sender,
            treasury,
            authorization.final_total_amount
        );

        order_id = next_order_id;
        next_order_id = order_id + 1;
        total_amount = authorization.final_total_amount;

        emit TicketsPurchasedWithIntent(
            order_id,
            authorization.intent_id,
            msg.sender,
            payment_token,
            total_amount,
            level_ids,
            quantities,
            unit_prices,
            block.timestamp
        );
    }

    function _verifyAndConsumeIntent(
        address payment_token,
        uint8[] calldata level_ids,
        uint256[] calldata quantities,
        PurchaseAuthorization memory authorization,
        uint256 quoted_total_amount
    ) internal {
        if (purchase_signer == address(0)) {
            revert PurchaseSignerMissing();
        }
        if (uint64(block.timestamp) > authorization.expires_at) {
            revert AuthorizationExpired(
                authorization.expires_at,
                uint64(block.timestamp)
            );
        }
        if (consumed_intents[authorization.intent_id]) {
            revert IntentAlreadyConsumed(authorization.intent_id);
        }
        if (authorization.final_total_amount > quoted_total_amount) {
            revert FinalAmountExceedsQuotedTotal(
                authorization.final_total_amount,
                quoted_total_amount
            );
        }

        address recovered_signer = ECDSA.recover(
            MessageHashUtils.toEthSignedMessageHash(
                _buildAuthorizationDigest(
                    payment_token,
                    level_ids,
                    quantities,
                    authorization.intent_id,
                    authorization.final_total_amount,
                    authorization.expires_at
                )
            ),
            authorization.signature
        );
        if (recovered_signer != purchase_signer) {
            revert InvalidPurchaseSigner(recovered_signer);
        }

        consumed_intents[authorization.intent_id] = true;
    }

    function _buildAuthorizationDigest(
        address payment_token,
        uint8[] calldata level_ids,
        uint256[] calldata quantities,
        bytes32 intent_id,
        uint256 final_total_amount,
        uint64 expires_at
    ) internal view returns (bytes32) {
        uint256[] memory level_ids_u256 = new uint256[](level_ids.length);
        for (uint256 i = 0; i < level_ids.length; i++) {
            level_ids_u256[i] = uint256(level_ids[i]);
        }

        return keccak256(
            abi.encode(
                address(this),
                block.chainid,
                msg.sender,
                payment_token,
                level_ids_u256,
                quantities,
                intent_id,
                final_total_amount,
                uint256(expires_at)
            )
        );
    }

    function _quoteCurrentOrder(
        uint8[] calldata level_ids,
        uint256[] calldata quantities
    ) internal view returns (uint256 total_amount, uint256[] memory unit_prices) {
        _validateOrderInput(level_ids, quantities);

        uint64 now_timestamp = uint64(block.timestamp);
        unit_prices = new uint256[](level_ids.length);

        for (uint256 i = 0; i < level_ids.length; i++) {
            uint256 quantity = quantities[i];
            if (quantity == 0) {
                revert ZeroQuantity(i);
            }

            uint256 unit_price = _resolveCurrentPrice(
                level_ids[i],
                now_timestamp
            );
            unit_prices[i] = unit_price;
            total_amount += unit_price * quantity;
        }

        if (total_amount == 0) {
            revert EmptyOrder();
        }
    }

    function _resolveCurrentPrice(
        uint8 level_id,
        uint64 now_timestamp
    ) internal view returns (uint256) {
        _validateLevel(level_id);

        PriceSchedule storage schedule = _price_schedules[level_id];
        uint256 n = schedule.start_timestamps.length;

        if (n == 0) {
            revert PriceScheduleMissing(level_id);
        }
        if (now_timestamp < schedule.start_timestamps[0]) {
            revert PriceNotStarted(level_id, now_timestamp);
        }

        uint256 left = 0;
        uint256 right = n;
        while (left + 1 < right) {
            uint256 mid = (left + right) / 2;
            if (schedule.start_timestamps[mid] <= now_timestamp) {
                left = mid;
            } else {
                right = mid;
            }
        }

        return schedule.prices[left];
    }

    function _validateLevel(uint8 level_id) internal pure {
        if (level_id < MIN_LEVEL || level_id > MAX_LEVEL) {
            revert InvalidLevel(level_id);
        }
    }

    function _validateOrderInput(
        uint8[] calldata level_ids,
        uint256[] calldata quantities
    ) internal pure {
        if (level_ids.length == 0) {
            revert EmptyOrder();
        }
        if (level_ids.length != quantities.length) {
            revert MismatchedOrderInputLength();
        }
    }

    modifier nonReentrant() {
        if (_reentrancy_lock == 1) {
            revert ReentrancyBlocked();
        }
        _reentrancy_lock = 1;
        _;
        _reentrancy_lock = 0;
    }
}
