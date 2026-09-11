-- llm: store the sk- plaintext reversibly encrypted (APP_KEY) for
-- later reveal/copy in the admin UI — same pattern as api_token.
ALTER TABLE llm_tokens ADD COLUMN key_enc TEXT;
