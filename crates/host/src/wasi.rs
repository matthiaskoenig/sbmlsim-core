//! Minimal preview1 policy, matching the JavaScript loader. No ambient capabilities.
use crate::{HostState, WASI_MODULE};
use wasmtime::{Caller, ExternType, Linker, Memory, Module};

#[derive(Default)]
pub(crate) struct Output {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn memory(caller: &mut Caller<'_, HostState>) -> wasmtime::Result<Memory> {
    caller
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .ok_or_else(|| wasmtime::format_err!("WASI caller does not export memory"))
}
fn word(data: &[u8], at: usize) -> wasmtime::Result<u32> {
    let bytes = data
        .get(
            at..at
                .checked_add(4)
                .ok_or_else(|| wasmtime::format_err!("WASI pointer overflow"))?,
        )
        .ok_or_else(|| wasmtime::format_err!("WASI pointer outside memory"))?;
    Ok(u32::from_le_bytes(bytes.try_into()?))
}

pub(crate) fn add_to_linker(linker: &mut Linker<HostState>) -> wasmtime::Result<()> {
    linker.func_wrap(
        WASI_MODULE,
        "fd_write",
        |mut c: Caller<'_, HostState>,
         fd: i32,
         iovs: u32,
         count: u32,
         written: u32|
         -> wasmtime::Result<i32> {
            if fd != 1 && fd != 2 {
                return Ok(8);
            }
            // The loop runs in the host, where the guest's epoch deadline cannot interrupt
            // it. Bound the work even when every guest iovec is empty.
            if count > 1024 {
                return Ok(28);
            }
            let mem = memory(&mut c)?;
            let data = mem.data(&c);
            const LIMIT: usize = 1024 * 1024;
            let mut bytes = Vec::new();
            let mut total = 0u32;
            for i in 0..count {
                let at = (iovs as usize)
                    .checked_add(i as usize * 8)
                    .ok_or_else(|| wasmtime::format_err!("WASI pointer overflow"))?;
                let ptr = word(data, at)? as usize;
                let len = word(data, at + 4)? as usize;
                let end = ptr
                    .checked_add(len)
                    .ok_or_else(|| wasmtime::format_err!("WASI pointer overflow"))?;
                let chunk = data
                    .get(ptr..end)
                    .ok_or_else(|| wasmtime::format_err!("WASI buffer outside memory"))?;
                total = total
                    .checked_add(len as u32)
                    .ok_or_else(|| wasmtime::format_err!("WASI byte count overflow"))?;
                bytes.extend_from_slice(
                    &chunk[..chunk.len().min(LIMIT.saturating_sub(bytes.len()))],
                );
            }
            mem.write(&mut c, written as usize, &total.to_le_bytes())?;
            // Capture diagnostics locally, never inherit the process's descriptors.
            let output = &mut c.data_mut().wasi;
            let target = if fd == 2 {
                &mut output.stderr
            } else {
                &mut output.stdout
            };
            // Diagnostics must not defeat the instance's memory bound.
            target.extend_from_slice(&bytes[..bytes.len().min(LIMIT.saturating_sub(target.len()))]);
            Ok(0)
        },
    )?;
    linker.func_wrap(WASI_MODULE, "environ_get", |_: i32, _: i32| -> i32 { 0 })?;
    linker.func_wrap(
        WASI_MODULE,
        "environ_sizes_get",
        |mut c: Caller<'_, HostState>, count: u32, size: u32| -> wasmtime::Result<i32> {
            let mem = memory(&mut c)?;
            mem.write(&mut c, count as usize, &0u32.to_le_bytes())?;
            mem.write(&mut c, size as usize, &0u32.to_le_bytes())?;
            Ok(0)
        },
    )?;
    linker.func_wrap(
        WASI_MODULE,
        "proc_exit",
        |code: i32| -> wasmtime::Result<()> {
            wasmtime::bail!("engine.wasm called proc_exit({code})")
        },
    )?;
    linker.func_wrap(
        WASI_MODULE,
        "clock_time_get",
        |_: i32, _: i64, _: i32| -> wasmtime::Result<i32> {
            wasmtime::bail!("engine.wasm called WASI clock_time_get: the engine reads no clock")
        },
    )?;
    linker.func_wrap(
        WASI_MODULE,
        "random_get",
        |_: i32, _: i32| -> wasmtime::Result<i32> {
            wasmtime::bail!("engine.wasm called WASI random_get: the engine draws no random bytes")
        },
    )?;
    Ok(())
}

/// Unknown functions keep their exact declared signature but always trap on invocation.
pub(crate) fn reject_other_imports(
    linker: &mut Linker<HostState>,
    module: &Module,
) -> wasmtime::Result<()> {
    let mut seen = std::collections::HashSet::new();
    for import in module.imports() {
        if import.module() != WASI_MODULE || !seen.insert(import.name().to_owned()) {
            continue;
        }
        if matches!(
            import.name(),
            "fd_write"
                | "environ_get"
                | "environ_sizes_get"
                | "proc_exit"
                | "clock_time_get"
                | "random_get"
        ) {
            continue;
        }
        let ExternType::Func(ty) = import.ty() else {
            wasmtime::bail!("WASI import is not a function");
        };
        let name = import.name().to_owned();
        linker.func_new(WASI_MODULE, import.name(), ty, move |_, _, _| {
            wasmtime::bail!("engine.wasm called unavailable WASI {name}")
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store;

    fn run(imports: &str, body: &str) -> wasmtime::Result<crate::HostState> {
        let engine = wasmtime::Engine::default();
        let wasm = wat::parse_str(format!(
            "(module {imports} (memory (export \"memory\") 1) (func (export \"run\") {body}))"
        ))?;
        let module = Module::new(&engine, wasm)?;
        let mut linker = Linker::new(&engine);
        add_to_linker(&mut linker)?;
        reject_other_imports(&mut linker, &module)?;
        let mut store = store(&engine, 65536);
        let instance = linker.instantiate(&mut store, &module)?;
        store.data_mut().memory.grows = 0;
        instance
            .get_typed_func::<(), ()>(&mut store, "run")?
            .call(&mut store, ())?;
        Ok(store.into_data())
    }

    #[test]
    fn environment_is_empty_and_calls_are_counted() {
        let state = run(r#"(import "wasi_snapshot_preview1" "environ_sizes_get" (func $env (param i32 i32) (result i32)))"#,
            "i32.const 0 i32.const 4 call $env drop i32.const 0 i32.load if unreachable end i32.const 4 i32.load if unreachable end").unwrap();
        assert_eq!(state.wasi_calls(), 1);
    }

    #[test]
    fn stdout_is_captured_and_other_descriptors_rejected() {
        let state = run(r#"(import "wasi_snapshot_preview1" "fd_write" (func $write (param i32 i32 i32 i32) (result i32)))"#,
            "i32.const 0 i32.const 32 i32.store i32.const 4 i32.const 1 i32.store i32.const 32 i32.const 65 i32.store8 i32.const 1 i32.const 0 i32.const 1 i32.const 8 call $write if unreachable end i32.const 8 i32.load i32.const 1 i32.ne if unreachable end i32.const 3 i32.const 0 i32.const 1 i32.const 8 call $write i32.const 8 i32.ne if unreachable end").unwrap();
        assert_eq!(state.wasi.stdout, b"A");
        assert_eq!(state.wasi_calls(), 2);
    }

    #[test]
    fn writes_reject_unbounded_host_work_and_invalid_buffers() {
        let imports = r#"(import "wasi_snapshot_preview1" "fd_write" (func $write (param i32 i32 i32 i32) (result i32)))"#;
        run(imports, "i32.const 1 i32.const 0 i32.const -1 i32.const 8 call $write i32.const 28 i32.ne if unreachable end").unwrap();
        assert!(
            run(
                imports,
                "i32.const 1 i32.const 65532 i32.const 1 i32.const 8 call $write drop"
            )
            .is_err()
        );
    }

    #[test]
    fn ambient_capabilities_and_exit_trap_with_exact_signatures() {
        for (name, params, args, result) in [
            (
                "clock_time_get",
                "i32 i64 i32",
                "i32.const 0 i64.const 0 i32.const 0",
                "(result i32)",
            ),
            (
                "random_get",
                "i32 i32",
                "i32.const 0 i32.const 8",
                "(result i32)",
            ),
            (
                "path_open",
                "i32 i32 i32 i32 i32 i64 i64 i32 i32",
                "i32.const 0 i32.const 0 i32.const 0 i32.const 0 i32.const 0 i64.const 0 i64.const 0 i32.const 0 i32.const 0",
                "(result i32)",
            ),
            (
                "sock_accept",
                "i32 i32 i32",
                "i32.const 0 i32.const 0 i32.const 0",
                "(result i32)",
            ),
            ("proc_exit", "i32", "i32.const 0", ""),
        ] {
            let imports = format!(
                r#"(import "wasi_snapshot_preview1" "{name}" (func $call (param {params}) {result}))"#
            );
            let body = format!(
                "{args} call $call {}",
                if result.is_empty() { "" } else { "drop" }
            );
            let error = match run(&imports, &body) {
                Ok(_) => panic!("{name} did not trap"),
                Err(error) => error,
            };
            assert!(format!("{error:#}").contains(name), "{error:#}");
        }
    }
}
