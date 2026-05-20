# Mail Delivery Reliability

The backend supports two mail providers:

- `console`: log only (local/dev)
- `webhook`: POST mail payload to a configured webhook
- `resend`: send branded ticket emails through Resend

## Retry Strategy

For `MAIL_PROVIDER=webhook` and `MAIL_PROVIDER=resend`, ticket emails are retried on failure.

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
- `MAIL_REPLY_TO`
- `MAIL_PROVIDER`
- `MAIL_WEBHOOK_URL`
- `MAIL_API_KEY`
- `MAIL_MAX_RETRIES`
- `MAIL_RETRY_BACKOFF_MS`
- `MAIL_ALERT_WEBHOOK_URL`
- `MAIL_ALERT_API_KEY`
- `APP_PUBLIC_BASE_URL`
- `EMAIL_ACCESS_TOKEN_TTL_SECS`
- `EMAIL_SESSION_TTL_HOURS`

For Resend, set:

- `MAIL_PROVIDER=resend`
- `MAIL_API_KEY=<resend api key>`
- `MAIL_FROM=Money Frontier Tickets <tickets@moneyfrontier.info>`
- `MAIL_REPLY_TO=contact@moneyfrontier.info`

Email ticket access uses a one-time link built from `APP_PUBLIC_BASE_URL`. The link token is stored as a hash and can only be consumed once. After verification, the API returns a short-lived email ticket session token for reading `/tickets`.
