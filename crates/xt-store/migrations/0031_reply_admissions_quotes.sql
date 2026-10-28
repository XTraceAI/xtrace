-- Admission phrases inside a closed quotation no longer count (reply-admission
-- fact version 2, src/reply_admission.rs): a reply that quotes "you're right"
-- does not admit a mistake. Rows saved under version 1 are replaced when their
-- reply is read again. Clear the Claude transcript checkpoints once, so the
-- next native scan reads every Claude transcript again (as migration 30 did).
-- Codex and Cursor are read whole by the pinned reader on every scan, so they
-- are replaced without a reset. Nothing else changes.
DELETE FROM native_checkpoints WHERE source='transcript';
