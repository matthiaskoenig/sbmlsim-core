"""The allocator the extension module links, which is not the same on every platform."""

import importlib.machinery
import sys
from pathlib import Path

import sbmlsim_core

#: a message of mimalloc's own C sources, which a release build keeps although it strips the
#: symbols. Not the bare name: the extension embeds `THIRD_PARTY.md` as `THIRD_PARTY`, which
#: names mimalloc in prose on every platform, so the bare name is in the bytes of the macOS
#: wheel as well
MIMALLOC = b"mimalloc: error: "


def test_the_macos_wheel_carries_no_fixed_slot_allocator() -> None:
    """The macOS build allocates through the system allocator, every other one through mimalloc.

    On macOS mimalloc keeps the heap of a thread in two slots of the pthread TLS block which it
    claims by compiled in number, so two extension modules which each link a copy of it hand
    each other their heaps and corrupt them. The test suite of `sbmlsim`, which loads this
    module next to `pkpdutils_core`, failed on macOS and only on macOS while both linked one
    (sbmlsim#565, #40); the `mod allocator` comment of `crates/python/src/lib.rs` is the long
    version.

    Both halves are asserted, so this fails if a build loses mimalloc where it is wanted as well
    as if it gains one where it corrupts: the attribute is what the compiled code says about
    itself and the bytes of the extension are what it actually links. The two halves are not
    equally strong, since finding the message proves it is linked while not finding it only
    fails to prove it, so the macOS half rests on `__allocator__`, which is exact, and the byte
    search is what makes the other platforms assert anything at all.
    """
    linked = sys.platform != "darwin"
    assert sbmlsim_core.__allocator__ == ("mimalloc" if linked else "system")

    suffixes = tuple(importlib.machinery.EXTENSION_SUFFIXES)
    extensions = [
        path
        for directory in sbmlsim_core.__path__
        for path in sorted(Path(directory).iterdir())
        if path.name.endswith(suffixes)
    ]
    assert len(extensions) == 1, f"one extension module, not {extensions}"
    assert (MIMALLOC in extensions[0].read_bytes()) is linked, (
        f"{extensions[0].name} and __allocator__ disagree about mimalloc"
    )
