-- What the two Claude `Bash` launch readers last found in a Claude
-- transcript file that held no launch, so that a restart need not read an
-- unchanged file again: one row per reader and file.
--
-- `reader` is the reader: `prompt` (the literal-prompt launch match) or
-- `bash` (the launch reader that compares printed answers). A row is kept
-- only while what it was read with is what reads it now: the same
-- `reader_version`, longest line `max_line` and `rules_digest`, a digest of
-- the readers' own rules and of the Claude launcher programs installed when
-- the read began. It names the file by the path the index's own source
-- cursors already hold, the session owning it, and the generation the read
-- began at: device, inode and exact modification and change times, and
-- `read_to`, the byte just after the last complete line read. `fingerprint`
-- is a SHA-256 digest of `read_to` and the bytes of the first and last 4 KiB
-- before it. For `prompt`, `untimed` and `refused_ms` keep what the read
-- found that is not a launch: whether a record or line naming `claude` had no
-- time, and the times (UTC milliseconds, each eight little-endian bytes) of
-- `Bash` commands naming `claude` it could not read. No prompt, command,
-- record text or output is kept. A row is only a shortcut: a file whose
-- generation or fingerprint differs is read as if no row existed. Additive
-- only: no existing table changes.
CREATE TABLE claude_transcript_reads (
    reader TEXT NOT NULL CHECK(reader IN ('prompt', 'bash')),
    path TEXT NOT NULL CHECK(
        typeof(path) = 'text' AND length(path) BETWEEN 2 AND 4096 AND substr(path, 1, 1) = '/'
    ),
    owner_native_session_id TEXT NOT NULL CHECK(
        typeof(owner_native_session_id) = 'text'
        AND length(trim(owner_native_session_id)) > 0
        AND length(owner_native_session_id) <= 256
    ),
    reader_version INTEGER NOT NULL CHECK(
        typeof(reader_version) = 'integer' AND reader_version >= 1
    ),
    max_line INTEGER NOT NULL CHECK(typeof(max_line) = 'integer' AND max_line >= 1),
    rules_digest BLOB NOT NULL CHECK(
        typeof(rules_digest) = 'blob' AND length(rules_digest) = 32
    ),
    device INTEGER NOT NULL CHECK(typeof(device) = 'integer'),
    inode INTEGER NOT NULL CHECK(typeof(inode) = 'integer'),
    read_to INTEGER NOT NULL CHECK(typeof(read_to) = 'integer' AND read_to >= 0),
    mtime_ns INTEGER NOT NULL CHECK(typeof(mtime_ns) = 'integer'),
    ctime_ns INTEGER NOT NULL CHECK(typeof(ctime_ns) = 'integer'),
    fingerprint BLOB NOT NULL CHECK(
        typeof(fingerprint) = 'blob' AND length(fingerprint) = 32
    ),
    untimed INTEGER NOT NULL CHECK(
        untimed IN (0, 1) AND (reader = 'prompt' OR untimed = 0)
    ),
    refused_ms BLOB NOT NULL CHECK(
        typeof(refused_ms) = 'blob' AND length(refused_ms) % 8 = 0
        AND length(refused_ms) <= 80000
        AND (reader = 'prompt' OR length(refused_ms) = 0)
    ),
    saved_at INTEGER NOT NULL CHECK(typeof(saved_at) = 'integer' AND saved_at >= 0),
    PRIMARY KEY(reader, path)
) WITHOUT ROWID;
