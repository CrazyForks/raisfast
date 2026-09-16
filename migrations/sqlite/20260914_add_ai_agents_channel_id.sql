-- llm: pin an agent to a specific upstream channel (design §10.1 `pin_channel`).
ALTER TABLE ai_agents ADD COLUMN channel_id INTEGER;
