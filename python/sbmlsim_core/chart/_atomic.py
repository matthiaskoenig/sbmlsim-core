"""Atomic writes of the files of a chart.

A file is written into a temporary file of its directory, which replaces it once
it is complete, so a reader sees the old or the new file and never a partial
one. The temporary file is created with the mode `0o666` and the umask of the
process, the mode of a file written with `open`.
"""

from __future__ import annotations

import os
import secrets
from pathlib import Path

#: attempts to find a name for the temporary file which does not exist
_ATTEMPTS = 100


def write_bytes_atomic(path: Path, content: bytes) -> None:
    """Write a file atomically.

    Args:
        path: the file to write; its directory must exist.
        content: the content of the file.

    Raises:
        FileExistsError: if no free name for the temporary file was found.
    """
    path = Path(path)
    for _ in range(_ATTEMPTS):
        temporary = path.parent / f".{path.name}.{secrets.token_hex(8)}.tmp"
        try:
            fd = os.open(temporary, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o666)
        except FileExistsError:
            continue
        break
    else:
        raise FileExistsError(f"No free name for a temporary file of '{path}'.")
    try:
        with os.fdopen(fd, "wb") as f:
            f.write(content)
        os.replace(temporary, path)
    except BaseException:
        temporary.unlink(missing_ok=True)
        raise


def write_text_atomic(path: Path, text: str) -> None:
    """Write a text file atomically in utf-8, see `write_bytes_atomic`.

    Args:
        path: the file to write; its directory must exist.
        text: the content of the file.
    """
    write_bytes_atomic(path, text.encode("utf-8"))
