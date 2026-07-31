-- Drop deprecated token_prefix column; token identity is now derived from the
-- AES-256-GCM encrypted full token. Run: raisfast db migrate
ALTER TABLE api_tokens DROP COLUMN token_prefix;
