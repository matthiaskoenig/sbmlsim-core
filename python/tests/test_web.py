"""The runtime and the inline build of the JavaScript host which the package ships for a page."""

import json

import sbmlsim_core
from sbmlsim_core import web


def _custom_sections(wasm: bytes) -> dict[str, bytes]:
    """The custom sections of a WebAssembly module by their name."""

    def leb(at: int) -> tuple[int, int]:
        value = shift = 0
        while True:
            byte = wasm[at]
            at += 1
            value |= (byte & 0x7F) << shift
            shift += 7
            if byte < 0x80:
                return value, at

    sections, at = {}, 8
    while at < len(wasm):
        kind = wasm[at]
        size, start = leb(at + 1)
        if kind == 0:
            length, name_start = leb(start)
            name = wasm[name_start : name_start + length].decode()
            sections[name] = wasm[name_start + length : start + size]
        at = start + size
    return sections


def test_the_runtime_is_the_engine_and_the_math_library() -> None:
    files = sbmlsim_core.runtime_wasm()
    assert sorted(files) == ["engine", "mathlib"]
    for wasm in files.values():
        assert wasm[:4] == b"\0asm"
    section = json.loads(_custom_sections(files["engine"])["sbmlsim.engine"])
    assert set(section) == {"memory_minimum_pages", "output_format"}


def test_the_notices_of_the_engine_name_its_libraries() -> None:
    for library in ("SUNDIALS", "KLU"):
        assert library in sbmlsim_core.THIRD_PARTY


def test_the_inline_build_is_one_classic_script_of_this_release() -> None:
    text = web.script()
    assert text.startswith(
        f"// sbmlsim-core {sbmlsim_core.__version__}, the inline build"
    )
    assert "var sbmlsimCore" in text
    # a classic script: no module syntax, no URL of a module
    for module_syntax in ("import.meta", "\nexport ", "\nimport "):
        assert module_syntax not in text
    assert f'"{sbmlsim_core.__version__}"' in text
