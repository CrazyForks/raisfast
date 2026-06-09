-- Auto-generated migration to fix column type mismatches
-- Table: newsletter_subscribers, Content type: newsletter_subscriber
-- Mismatches:
--   email:  -> VARCHAR(255)
--   created_at:  -> DATETIME
--   updated_at:  -> DATETIME
--
-- REVIEW THIS FILE BEFORE EXECUTING!
-- Data in mismatched columns may be lost or truncated during type conversion.
-- Back up your database before running.
--

SET FOREIGN_KEY_CHECKS = 0;

CREATE TABLE newsletter_subscribers__new (
    id BIGINT PRIMARY KEY,
    email VARCHAR(255) NOT NULL,
    created_at DATETIME,
    updated_at DATETIME
);


INSERT INTO newsletter_subscribers__new SELECT * FROM newsletter_subscribers;

DROP TABLE newsletter_subscribers;

ALTER TABLE newsletter_subscribers__new RENAME TO newsletter_subscribers;

CREATE UNIQUE INDEX idx_newsletter_subscribers_email_unique ON newsletter_subscribers(email);
CREATE UNIQUE INDEX idx_newsletter_subscribers_email_unique ON newsletter_subscribers(email);

SET FOREIGN_KEY_CHECKS = 1;
