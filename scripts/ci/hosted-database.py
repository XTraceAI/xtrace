"""Check an owned, stopped app's database. Never repair it or print its contents."""
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import stat
import subprocess
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


def apple_diagnostic(arguments):
    try:
        result = subprocess.run(['/usr/bin/sqlite3', *arguments], capture_output=True, text=True, timeout=5)
        return dict(exit=result.returncode, stdout=result.stdout[:1024], stderr=result.stderr[:1024])
    except (OSError, subprocess.TimeoutExpired) as error:
        return dict(error=type(error).__name__)


def check(path):
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
        # These diagnostics run on the original sidecar state. They never decide
        # acceptance or trigger a fallback/repair; Python is always the verifier.
        report['appleVersion'] = apple_diagnostic(['--version'])
        report['appleCheck'] = apple_diagnostic(['-readonly', str(path),
            'PRAGMA integrity_check; SELECT count(*) FROM sessions; SELECT count(*) FROM records;'])
        report['filesBeforePython'] = {key: metadata(value) for key, value in paths.items()}
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
    if len(sys.argv) != 2:
        raise SystemExit('Expected one owned database path')
    result = check(Path(sys.argv[1]))
    print(json.dumps(result, sort_keys=True))
    raise SystemExit(0 if result['status'] == 'passed' else 1)
