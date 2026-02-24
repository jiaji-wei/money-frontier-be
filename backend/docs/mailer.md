# Mail Delivery Reliability

The backend supports two mail providers:

- `console`: log only (local/dev)
- `webhook`: POST mail payload to a configured webhook

## Retry Strategy

For `MAIL_PROVIDER=webhook`, transfer emails are retried on failure.

- Total attempts = `MAIL_MAX_RETRIES + 1`
- Delay between attempts = `MAIL_RETRY_BACKOFF_MS`

If all attempts fail, transfer API returns error.

## Alerting

Optional alert webhook can be configured for exhausted retries:

- `MAIL_ALERT_WEBHOOK_URL`
- `MAIL_ALERT_API_KEY`

When configured, backend sends `ticket_email_delivery_failed` alert payload after retries are exhausted.

## Config

- `MAIL_FROM`
- `MAIL_PROVIDER`
- `MAIL_WEBHOOK_URL`
- `MAIL_API_KEY`
- `MAIL_MAX_RETRIES`
- `MAIL_RETRY_BACKOFF_MS`
- `MAIL_ALERT_WEBHOOK_URL`
- `MAIL_ALERT_API_KEY`
