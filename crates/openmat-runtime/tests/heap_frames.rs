//! These tests deliberately use the Windows executable's ordinary 1 MiB stack.
//! Compiling source happens outside that thread: this tests the VM, not parser
//! recursion. No `RUST_MIN_STACK` or linker `/STACK` workaround is involved.

use std::sync::{Arc, Mutex};

use openmat_bytecode::BytecodeModule;
use openmat_runtime::{
    BuiltinContext, BuiltinRegistry, Interpreter, RuntimeConfig, RuntimeErrorKind,
};
use openmat_source::SourceId;
use openmat_value::Value;

fn compile(source: &str) -> BytecodeModule {
    let parsed = openmat_parser::parse(SourceId::new(1), source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let lowered = openmat_hir::lower(&parsed.syntax);
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
    openmat_compiler::compile(&lowered.file).expect("test source compiles")
}

fn on_small_stack(test: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .name("heap-m-frames".into())
        .stack_size(1024 * 1024)
        .spawn(test)
        .unwrap()
        .join()
        .unwrap();
}

fn expect_result(runtime: &Interpreter, expected: f64) {
    assert_eq!(
        runtime.workspace().get("result"),
        Some(&Value::Double(expected))
    );
}

#[test]
fn default_limit_allows_2048_frames_with_constant_native_stack_usage() {
    let module = compile(
        // One script frame plus descend(2046) through descend(0): 2048 frames.
        "result = descend(2046);\n\
         function y = descend(n)\n\
             probe();\n\
             if n == 0\n y = 7;\n else\n y = 1 + descend(n - 1);\n end\n\
         end",
    );
    on_small_stack(move || {
        let addresses = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&addresses);
        let mut builtins = BuiltinRegistry::new();
        builtins
            .register(
                "probe",
                move |_args: &[Value], _: &mut BuiltinContext<'_>| {
                    let marker = 0_u8;
                    // Inspect the address, without dereferencing it or using unsafe.
                    observed
                        .lock()
                        .unwrap()
                        .push(std::ptr::from_ref(&marker) as usize);
                    Ok(Vec::new())
                },
            )
            .unwrap();
        let mut runtime = Interpreter::with_registry(module, builtins).unwrap();
        assert_eq!(runtime.config().maximum_call_depth, 2048);
        runtime.execute_entry(&[]).unwrap();
        expect_result(&runtime, 2053.0);
        let addresses = addresses.lock().unwrap();
        assert_eq!(addresses.len(), 2047);
        let span = addresses.iter().max().unwrap() - addresses.iter().min().unwrap();
        assert!(
            span < 16 * 1024,
            "native stack grew with m depth: {span} bytes"
        );
    });
}

#[test]
fn mutual_recursion_and_feval_preserve_multiple_results() {
    let module = compile(
        "[a,b] = evenStep(600); result = a+b;\n\
         function [a,b] = evenStep(n)\n\
           if n == 0\n a=2; b=3;\n else\n [a,b]=feval(@oddStep,n-1); a=a+1;\n end\n\
         end\n\
         function [a,b] = oddStep(n)\n [a,b]=evenStep(n-1); b=b+1;\n end",
    );
    on_small_stack(move || {
        let mut runtime = Interpreter::new(module).unwrap();
        runtime.set_config(RuntimeConfig {
            maximum_call_depth: 1024,
        });
        runtime.execute_entry(&[]).unwrap();
        expect_result(&runtime, 605.0);
    });
}

#[test]
fn depth_limit_has_a_complete_trace_and_session_can_run_again() {
    let module = compile("result = recur();\n function y = recur()\n y = recur();\n end");
    let next = compile("result = 42;");
    on_small_stack(move || {
        let mut runtime = Interpreter::new(module).unwrap();
        let error = runtime.execute_entry(&[]).unwrap_err();
        assert_eq!(
            error.kind,
            RuntimeErrorKind::CallDepthExceeded { maximum: 2048 }
        );
        assert_eq!(error.stack.len(), 2048);
        assert!(
            error
                .stack
                .iter()
                .skip(1)
                .all(|frame| frame.name.ends_with("recur"))
        );
        // Hosts can still impose a smaller budget, and an exhausted driver
        // must not leave frames behind for the next invocation.
        runtime.set_config(RuntimeConfig {
            maximum_call_depth: 32,
        });
        let error = runtime.execute_entry(&[]).unwrap_err();
        assert_eq!(
            error.kind,
            RuntimeErrorKind::CallDepthExceeded { maximum: 32 }
        );
        assert_eq!(error.stack.len(), 32);
        runtime.replace_module(next).unwrap();
        runtime.execute_entry(&[]).unwrap();
        expect_result(&runtime, 42.0);
    });
}

#[test]
fn deep_exception_unwinds_to_the_correct_caller() {
    let module = compile(
        "result = outer();\n\
         function y = outer()\n\
           try\n y=descend(500);\n catch err\n y=41;\n end\n y=y+1;\n\
         end\n\
         function y = descend(n)\n\
           if n == 0\n y=missing_at_bottom;\n else\n y=descend(n-1);\n end\n\
         end",
    );
    on_small_stack(move || {
        let mut runtime = Interpreter::new(module).unwrap();
        runtime.set_config(RuntimeConfig {
            maximum_call_depth: 1024,
        });
        runtime.execute_entry(&[]).unwrap();
        expect_result(&runtime, 42.0);
    });
}

#[test]
fn eval_reuses_each_suspended_callers_own_scope() {
    let module = compile(
        "result=descend(200);\n\
         function y=descend(n)\n\
           saved=n;\n\
           if n==0\n y=3;\n else\n y=eval('descend(n-1)')+saved;\n end\n\
         end",
    );
    on_small_stack(move || {
        let mut runtime = Interpreter::new(module).unwrap();
        runtime.set_config(RuntimeConfig {
            maximum_call_depth: 512,
        });
        runtime.execute_entry(&[]).unwrap();
        expect_result(&runtime, 20103.0);
        assert!(!runtime.workspace().contains("saved"));
    });
}

#[test]
fn native_callback_can_enter_a_deep_heap_call_chain() {
    let module = compile(
        "result = bridge(@descend, 500);\n\
         function y = descend(n)\n\
           if n == 0\n y=5;\n else\n y=descend(n-1)+1;\n end\n\
         end",
    );
    on_small_stack(move || {
        let mut builtins = BuiltinRegistry::new();
        builtins
            .register(
                "bridge",
                |args: &[Value], context: &mut BuiltinContext<'_>| {
                    context.invoke(&args[0], &args[1..], 1)
                },
            )
            .unwrap();
        let mut runtime = Interpreter::with_registry(module, builtins).unwrap();
        runtime.set_config(RuntimeConfig {
            maximum_call_depth: 1024,
        });
        runtime.execute_entry(&[]).unwrap();
        expect_result(&runtime, 505.0);
    });
}

#[test]
fn repeated_native_reentry_fails_structurally_and_restores_the_session() {
    let module = compile(
        "result = descend(1000);\n\
         function y = descend(n)\n\
           if n == 0\n y=5;\n else\n y=bridge(@descend,n-1)+1;\n end\n\
         end",
    );
    let next = compile("result = bridge(@identity, 42);\n function y=identity(x)\n y=x;\n end");
    on_small_stack(move || {
        let mut builtins = BuiltinRegistry::new();
        builtins
            .register(
                "bridge",
                |args: &[Value], context: &mut BuiltinContext<'_>| {
                    context.invoke(&args[0], &args[1..], 1)
                },
            )
            .unwrap();
        let mut runtime = Interpreter::with_registry(module, builtins).unwrap();
        runtime.set_config(RuntimeConfig {
            maximum_call_depth: 2048,
        });
        let error = runtime.execute_entry(&[]).unwrap_err();
        assert_eq!(
            error.kind,
            RuntimeErrorKind::CallDepthExceeded { maximum: 16 }
        );
        runtime.replace_module(next).unwrap();
        runtime.execute_entry(&[]).unwrap();
        expect_result(&runtime, 42.0);
    });
}

#[test]
fn cancellation_at_depth_unwinds_without_running_catch_and_can_restart() {
    let module = compile(
        "result=0; try\n result=descend(500);\n catch\n result=-1;\n end\n\
         function y=descend(n)\n\
           if n==0\n stop_now(); y=0;\n else\n y=descend(n-1)+1;\n end\n\
         end",
    );
    let next = compile("result=42;");
    on_small_stack(move || {
        let mut builtins = BuiltinRegistry::new();
        builtins
            .register(
                "stop_now",
                |_: &[Value], context: &mut BuiltinContext<'_>| {
                    context.cancellation().cancel();
                    Ok(Vec::new())
                },
            )
            .unwrap();
        let mut runtime = Interpreter::with_registry(module, builtins).unwrap();
        runtime.set_config(RuntimeConfig {
            maximum_call_depth: 1024,
        });
        let error = runtime.execute_entry(&[]).unwrap_err();
        assert_eq!(error.kind, RuntimeErrorKind::Cancelled);
        assert!(error.stack.len() > 500);
        expect_result(&runtime, 0.0);
        runtime.cancellation_token().reset();
        runtime.replace_module(next).unwrap();
        runtime.execute_entry(&[]).unwrap();
        expect_result(&runtime, 42.0);
    });
}

#[test]
fn constructors_methods_and_property_getters_suspend_on_the_heap() {
    let class = compile(
        "classdef HeapChain < handle\n\
         properties\n Next = [];\n end\n\
         properties (Dependent)\n Depth\n end\n\
         methods\n\
           function obj = HeapChain(n)\n\
             if n > 0\n obj.Next = HeapChain(n-1);\n end\n\
           end\n\
           function y = get.Depth(obj)\n\
             if isa(obj.Next,'HeapChain')\n y=obj.Next.Depth+1;\n else\n y=0;\n end\n\
           end\n\
           function y = count(obj)\n\
             if isa(obj.Next,'HeapChain')\n y=obj.Next.count()+1;\n else\n y=0;\n end\n\
           end\n\
         end\n end",
    );
    let script = compile("chain = HeapChain(300); result = chain.Depth + chain.count();");
    on_small_stack(move || {
        let mut runtime = Interpreter::new(class).unwrap();
        runtime.set_config(RuntimeConfig {
            maximum_call_depth: 512,
        });
        runtime.execute_entry(&[]).unwrap();
        runtime.replace_module(script).unwrap();
        runtime.execute_entry(&[]).unwrap();
        expect_result(&runtime, 600.0);
        runtime.clear_session();
    });
}

#[test]
fn operator_overloads_do_not_recurse_through_the_native_dispatcher() {
    let class = compile(
        "classdef HeapSum\n\
         properties\n N=0;\n end\n\
         methods\n\
           function obj=HeapSum(n)\n obj.N=n;\n end\n\
           function y=plus(obj,rhs)\n\
             if obj.N==0\n y=rhs;\n else\n\
               previous=HeapSum(obj.N-1); y=(previous+rhs)+1;\n end\n\
           end\n\
         end\n end",
    );
    let script = compile("result=HeapSum(400)+2;");
    on_small_stack(move || {
        let mut runtime = Interpreter::new(class).unwrap();
        runtime.set_config(RuntimeConfig {
            maximum_call_depth: 512,
        });
        runtime.execute_entry(&[]).unwrap();
        runtime.replace_module(script).unwrap();
        runtime.execute_entry(&[]).unwrap();
        expect_result(&runtime, 402.0);
    });
}
