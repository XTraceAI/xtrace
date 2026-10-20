"""Check an owned, stopped app's database. Never repair it or print its contents."""
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import stat
import sys


def metadata(path):
    try:
        item = path.lstat()
    except FileNotFoundError:
        return None
    kind = 'file' if stat.S_ISREG(item.st_mode) else 'directory' if stat.S_ISDIR(item.st_mode) else 'unsafe'
    return dict(kind=kind, size=item.st_size, mode=stat.S_IMODE(item.st_mode), uid=item.st_uid,
                readable=os.access(path, os.R_OK), writable=os.access(path, os.W_OK),
                searchable=os.access(path, os.X_OK) if kind == 'directory' else None,
                sha256=digest(path) if kind == 'file' else None)


def digest(path):
    value = hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            value.update(chunk)
    return value.hexdigest()


def telemetry_facts(connection, expected_enabled):
    settings = dict(connection.execute("SELECT key,value_json FROM settings WHERE key IN ('telemetry.enabled','telemetry.disclosure_shown')"))
    if set(settings) != {'telemetry.enabled', 'telemetry.disclosure_shown'}:
        raise ValueError('Required saved telemetry settings are absent')
    if any(value not in ('true', 'false') for value in settings.values()):
        raise ValueError('Saved telemetry settings have an invalid value')
    enabled = settings['telemetry.enabled'] == 'true'
    disclosed = settings['telemetry.disclosure_shown'] == 'true'
    if enabled != expected_enabled or not disclosed:
        raise ValueError('Saved telemetry preference or disclosure does not match the phase')
    counts = {'pending': 0, 'failed': 0, 'sent': 0}
    events = {}
    for event, status, count in connection.execute('SELECT event,status,count(*) FROM telemetry_events GROUP BY event,status'):
        if event not in ('app_opened', 'daily_active', 'hourly_active') or status not in counts:
            raise ValueError('Unexpected telemetry ledger event or status')
        counts[status] += count
        events[event] = events.get(event, 0) + count
    total = sum(counts.values())
    if total > 200 or counts['sent']:
        raise ValueError('Telemetry ledger exceeded its bound or contains sent rows under the rejecting proxy')
    last_id = connection.execute('SELECT coalesce(max(id),0) FROM telemetry_events').fetchone()[0]
    return dict(preferenceEnabled=enabled, disclosureShown=disclosed,
                ledger=dict(rows=total, lastId=last_id, events=events, **counts))


def check(path, telemetry_enabled=None):
    report = dict(status='failed', python=sys.version.split()[0], sqlite=sqlite3.sqlite_version, mode='ro')
    paths = dict(parent=path.parent, database=path, wal=Path(str(path) + '-wal'), shm=Path(str(path) + '-shm'))
    connection = None
    before = None
    try:
        parent = metadata(path.parent)
        if parent is None or parent['kind'] != 'directory':
            raise ValueError('Missing or unsafe parent')
        report['filesBefore'] = {key: metadata(value) for key, value in paths.items()}
        for key, expected in [('parent', 'directory'), ('database', 'file'), ('wal', 'file'), ('shm', 'file')]:
            item = report['filesBefore'][key]
            if (item is None and key in ('parent', 'database')) or (item and item['kind'] != expected):
                raise ValueError('Missing or unsafe ' + key)
        before = report['mainSha256Before'] = digest(path)
        with path.open('rb') as stream:
            header = stream.read(20)
        report['header'] = dict(sqliteMagic=header[:16] == b'SQLite format 3\0',
                                pageSizeField=int.from_bytes(header[16:18], 'big'),
                                writeVersion=header[18] if len(header) > 18 else None,
                                readVersion=header[19] if len(header) > 19 else None)
        report['stage'] = 'open'
        connection = sqlite3.connect(path.absolute().as_uri() + '?mode=ro', uri=True, timeout=5)
        report['stage'] = 'integrity'
        report['integrity'] = [row[0] for row in connection.execute('PRAGMA integrity_check').fetchmany(101)]
        report['stage'] = 'sessions'
        report['sessions'] = connection.execute('SELECT count(*) FROM sessions').fetchone()[0]
        report['stage'] = 'records'
        report['records'] = connection.execute('SELECT count(*) FROM records').fetchone()[0]
        if report['integrity'] != ['ok'] or report['sessions'] != 0 or report['records'] != 0:
            raise ValueError('Database integrity or empty-table acceptance failed')
        if telemetry_enabled is not None:
            report['stage'] = 'telemetry'
            report['telemetry'] = telemetry_facts(connection, telemetry_enabled)
        report['status'] = 'passed'
    except (OSError, ValueError, sqlite3.Error) as error:
        report['error'] = str(error)[:1024]
    finally:
        try:
            if connection is not None:
                connection.close()
            if before is not None:
                report['filesAfter'] = {key: metadata(value) for key, value in paths.items()}
                report['mainSha256After'] = digest(path)
                report['mainDatabaseUnchanged'] = before == report['mainSha256After']
                if not report['mainDatabaseUnchanged']:
                    report.update(status='failed', error='Main database bytes changed during verification')
        except (OSError, sqlite3.Error) as error:
            report.update(status='failed', error=str(error)[:1024])
    return report


if __name__ == '__main__':
    if len(sys.argv) not in (2, 3) or (len(sys.argv) == 3 and sys.argv[2] not in ('--telemetry=on', '--telemetry=off')):
        raise SystemExit('Expected one owned database path and optional explicit telemetry phase')
    enabled = sys.argv[2] == '--telemetry=on' if len(sys.argv) == 3 else None
    result = check(Path(sys.argv[1]), enabled)
    print(json.dumps(result, sort_keys=True))
    raise SystemExit(0 if result['status'] == 'passed' else 1)
