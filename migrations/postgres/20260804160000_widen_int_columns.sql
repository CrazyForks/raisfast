-- Widen integer columns to BIGINT so they decode as Rust `i64` on PostgreSQL.
--
-- sqlx maps Postgres `INTEGER` to `i32`; the model layer reads these columns
-- as `i64`, which previously failed with "Rust type `i64` not compatible with
-- SQL type `INT4`". Run: raisfast db migrate
ALTER TABLE posts ALTER COLUMN view_count TYPE BIGINT;
ALTER TABLE posts ALTER COLUMN reading_time TYPE BIGINT;
ALTER TABLE products ALTER COLUMN total_sales TYPE BIGINT;
ALTER TABLE products ALTER COLUMN version TYPE BIGINT;
ALTER TABLE currencies ALTER COLUMN version TYPE BIGINT;
ALTER TABLE payment_channels ALTER COLUMN version TYPE BIGINT;
ALTER TABLE payment_orders ALTER COLUMN version TYPE BIGINT;
ALTER TABLE workflow_definitions ALTER COLUMN version TYPE BIGINT;
ALTER TABLE sms_codes ALTER COLUMN attempts TYPE BIGINT;
ALTER TABLE wallet_outbox ALTER COLUMN attempts TYPE BIGINT;
ALTER TABLE wallet_outbox ALTER COLUMN max_attempts TYPE BIGINT;
ALTER TABLE cron_execution_log ALTER COLUMN duration_ms TYPE BIGINT;
