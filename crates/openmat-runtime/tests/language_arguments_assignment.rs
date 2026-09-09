use openmat_array::ArrayData;
use openmat_runtime::{Interpreter, RuntimeErrorKind};
use openmat_source::SourceId;
use openmat_value::Value;

fn compile(source: &str) -> openmat_bytecode::BytecodeModule {
    let parsed = openmat_parser::parse(SourceId::new(1), source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let hir = openmat_hir::lower(&parsed.syntax);
    assert!(hir.diagnostics.is_empty(), "{:?}", hir.diagnostics);
    openmat_compiler::compile(&hir.file).expect("source compiles")
}

fn execute(source: &str) -> Interpreter {
    let mut runtime = Interpreter::new(compile(source)).unwrap();
    runtime.execute_entry(&[]).expect("source executes");
    runtime
}

fn scalar(runtime: &Interpreter, name: &str, expected: f64) {
    assert_eq!(
        runtime.workspace().get(name),
        Some(&Value::Double(expected)),
        "{name}"
    );
}

fn array(runtime: &Interpreter, name: &str, expected: &[f64]) {
    let Some(Value::Array(ArrayData::F64(value))) = runtime.workspace().get(name) else {
        panic!(
            "expected double array {name}: {:?}",
            runtime.workspace().get(name)
        );
    };
    assert_eq!(value.as_slice(), expected, "{name}");
}

#[test]
fn fixed_inputs_can_be_omitted_and_initialized_without_changing_nargin() {
    let runtime = execute(
        "a=f(); b=f(2); c=f(2,4);\n\
        function y=f(x,y)\n n=nargin; if n<1; x=7; end; if n<2; y=3; end; y=x+y+100*nargin; end",
    );
    scalar(&runtime, "a", 10.0);
    scalar(&runtime, "b", 105.0);
    scalar(&runtime, "c", 206.0);
}

#[test]
fn omitted_fixed_inputs_also_work_with_varargin() {
    let runtime = execute(
        "a=f(); b=f(2); c=f(2,5);\n\
        function y=f(x,varargin)\n if nargin<1; x=7; end; y=x; if nargin>1; y=y+varargin{1}; end; end",
    );
    scalar(&runtime, "a", 7.0);
    scalar(&runtime, "b", 2.0);
    scalar(&runtime, "c", 7.0);
}

#[test]
fn handles_anonymous_functions_and_feval_allow_unused_omitted_inputs() {
    let runtime = execute(
        "h=@f; a=h(2); b=feval(h,3); g=@(x,y) x; c=g(4);\n\
        function y=f(x,unused)\n y=x; end",
    );
    scalar(&runtime, "a", 2.0);
    scalar(&runtime, "b", 3.0);
    scalar(&runtime, "c", 4.0);
}

#[test]
fn reading_a_missing_input_fails_at_the_read_and_is_catchable() {
    let mut runtime = Interpreter::new(compile(
        "result=f(2);\n\
        function z=f(x,y)\n global entered; entered=1; z=y; end",
    ))
    .unwrap();
    let error = runtime.execute_entry(&[]).unwrap_err();
    assert!(matches!(
        error.kind,
        RuntimeErrorKind::InputArity {
            actual: 1,
            expected: 2,
            ..
        }
    ));
    scalar(&runtime, "entered", 1.0);
    assert!(error.location.is_some());
    runtime
        .replace_module(compile(
            "caught=0; try; f(); catch err; caught=1; end;\n function z=f(x); z=x; end",
        ))
        .unwrap();
    runtime.execute_entry(&[]).unwrap();
    scalar(&runtime, "caught", 1.0);
}

#[test]
fn too_many_inputs_still_fail_before_entering_the_body() {
    let mut runtime = Interpreter::new(compile(
        "f(1,2); function f(x); global entered; entered=1; end",
    ))
    .unwrap();
    let error = runtime.execute_entry(&[]).unwrap_err();
    assert!(matches!(
        error.kind,
        RuntimeErrorKind::InputArity {
            actual: 2,
            expected: 1,
            ..
        }
    ));
    assert!(runtime.workspace().get("entered").is_none());
}

#[test]
fn missing_and_explicit_empty_inputs_are_distinct() {
    let runtime = execute("a=f(); b=f([]); function y=f(x); y=nargin; end");
    scalar(&runtime, "a", 0.0);
    scalar(&runtime, "b", 1.0);
}

#[test]
fn nested_shared_parameter_can_be_initialized_before_read() {
    let runtime = execute(
        "result=outer();\n function y=outer(x); init(); y=x;\n function init(); x=9; end; end",
    );
    scalar(&runtime, "result", 9.0);
}

#[test]
fn nested_end_uses_the_selected_array_for_growth() {
    let runtime =
        execute("C={[1,2,3]}; copy=C; C{1}(end)=9; C{1}(end+1)=4; result=C{1}; original=copy{1};");
    array(&runtime, "result", &[1.0, 2.0, 9.0, 4.0]);
    array(&runtime, "original", &[1.0, 2.0, 3.0]);
}

#[test]
fn nested_cell_end_deletion_preserves_the_original() {
    let runtime =
        execute("C={{1,2,3}}; copy=C; C{1}(end)=[]; result=C{1}{end}; original=copy{1}{end};");
    scalar(&runtime, "result", 2.0);
    scalar(&runtime, "original", 3.0);
}

#[test]
fn runtime_cell_and_struct_output_selectors_allow_growth() {
    let runtime = execute(
        "C={}; idx=[2,4]; [C{idx}]=pair(); result=[C{2},C{4}];\n\
        S.v=0; [S(idx).v]=pair(); fields=[S(2).v,S(4).v];\n function [x,y]=pair(); x=11;y=22;end",
    );
    array(&runtime, "result", &[11.0, 22.0]);
    array(&runtime, "fields", &[11.0, 22.0]);
}

#[test]
fn nested_struct_and_cell_end_binds_each_index_level_separately() {
    let runtime = execute("S.v={[1,2;3,4]}; C={S}; C{end}.v{end}(end, end)=8; result=C{1}.v{1};");
    array(&runtime, "result", &[1.0, 3.0, 2.0, 8.0]);
}

const PRODUCERS: &str = "\nfunction [x,y]=pair(); global log; log(end+1)=10+nargout; x=11; y=22; end\n\
    function x=mark(x); global log; log(end+1)=x; end\n\
    function [x,y,z]=triple(); global log; log(end+1)=10+nargout; x=11; y=22; z=33; end";

#[test]
fn fixed_indexed_outputs_evaluate_rhs_first_and_merge_repeated_roots() {
    // Independently observed in MATLAB R2022b: trace [12,1,2].
    let runtime = execute(&format!(
        "global log; log=[]; a=[0,0]; [a(mark(1)),a(mark(2))]=pair();{PRODUCERS}"
    ));
    array(&runtime, "a", &[11.0, 22.0]);
    array(&runtime, "log", &[12.0, 1.0, 2.0]);
}

#[test]
fn any_expanding_lhs_evaluates_all_selectors_before_rhs_once() {
    // Independently observed in MATLAB R2022b: trace [1,2,3,13].
    let runtime = execute(&format!(
        "global log; log=[]; a=[0,0]; C={{0,0}}; b=[0,0,0];\n\
        [a(mark(1)),C{{mark(2)}},b(mark(3))]=triple(); c=C{{2}};{PRODUCERS}"
    ));
    array(&runtime, "log", &[1.0, 2.0, 3.0, 13.0]);
    scalar(&runtime, "c", 22.0);
    array(&runtime, "a", &[11.0, 0.0]);
    array(&runtime, "b", &[0.0, 0.0, 33.0]);
}

#[test]
fn runtime_sized_cell_outputs_preserve_column_major_order_and_nargout() {
    let runtime = execute(
        "C={0,0,0,0}; idx=[1,3;2,4]; [C{idx}]=produce(); result=[C{1},C{2},C{3},C{4}];\n\
        function varargout=produce(); for k=1:nargout; varargout{k}=k; end; end",
    );
    array(&runtime, "result", &[1.0, 2.0, 3.0, 4.0]);
}

#[test]
fn mixed_dynamic_packs_names_and_discard_split_outputs_correctly() {
    let runtime = execute(
        "C={0,0,0}; idx=[3,1]; a=[0,0]; [C{idx},~,name,a(2)]=produce(); result=[C{1},C{2},C{3},name,a];\n\
        function varargout=produce(); for k=1:nargout; varargout{k}=k; end; end",
    );
    array(&runtime, "result", &[2.0, 0.0, 1.0, 4.0, 0.0, 5.0]);
}

#[test]
fn zero_destinations_request_zero_outputs_and_still_call_rhs() {
    let runtime = execute(&format!(
        "global log; log=[]; C={{1,2}}; [C{{[]}}]=pair(); result=[C{{1}},C{{2}}];{PRODUCERS}"
    ));
    array(&runtime, "log", &[10.0]);
    array(&runtime, "result", &[1.0, 2.0]);
}

#[test]
fn pack_writes_reload_roots_after_rhs_side_effects_and_previous_writes() {
    let runtime = execute(
        "global C; C={1,2,3}; [C{[1,2]}]=mutate(); result=[C{1},C{2},C{3}];\n\
        [C{1},C{2}]=mutate(); again=[C{1},C{2},C{3}];\n\
        function [x,y]=mutate(); global C; C={4,5,6}; x=11; y=22; end",
    );
    array(&runtime, "result", &[11.0, 22.0, 6.0]);
    array(&runtime, "again", &[11.0, 22.0, 6.0]);
}

#[test]
fn logical_colon_struct_fields_and_pack_rhs_are_supported() {
    let runtime = execute(
        "C={0,0,0}; mask=[true,false,true]; source={7,8}; [C{mask}]=source{:};\n\
        S.v=0; S(2).v=0; idx=[2,1]; [S(idx).v]=C{mask}; result=[S(1).v,S(2).v];\n [C{:}]=produce(); all=[C{1},C{2},C{3}];\n\
        function varargout=produce(); for k=1:nargout; varargout{k}=k; end; end",
    );
    array(&runtime, "result", &[8.0, 7.0]);
    array(&runtime, "all", &[1.0, 2.0, 3.0]);
}

#[test]
fn dynamic_output_errors_are_caught_and_do_not_poison_the_next_call() {
    let runtime = execute(
        "C={0,0}; caught=0; try; [C{:}]=one(); catch err; caught=1; end;\n\
        [C{:}]=pair(); result=[C{1},C{2}];\n function x=one(); x=7; end\n function [x,y]=pair(); x=8;y=9;end",
    );
    scalar(&runtime, "caught", 1.0);
    array(&runtime, "result", &[8.0, 9.0]);
}

#[test]
fn dynamic_output_function_handles_and_feval_observe_requested_count() {
    let runtime = execute(
        "C={0,0}; h=@produce; [C{:}]=h(5); a=[C{1},C{2}]; [C{:}]=feval(h,7); b=[C{1},C{2}];\n\
        function varargout=produce(x); for k=1:nargout; varargout{k}=x+k; end; end",
    );
    array(&runtime, "a", &[6.0, 7.0]);
    array(&runtime, "b", &[8.0, 9.0]);
}

#[test]
fn optional_methods_and_dynamic_method_and_static_dispatch_work() {
    let mut runtime = execute(
        "classdef OptionalBox\n properties\n X=3;\n end\n methods\n\
        function obj=OptionalBox(x); if nargin>0; obj.X=x; end; end\n\
        function varargout=produce(obj,x); if nargin<2; x=7; end; for k=1:nargout; varargout{k}=obj.X+x+k; end; end\n\
        end\n methods (Static)\n function [x,y]=pair(x); if nargin<1; x=11; end; y=22; end\n end\n end",
    );
    runtime
        .replace_module(compile(
            "obj=OptionalBox(); C={0,0}; [C{:}]=obj.produce(); result=[C{1},C{2}];\n\
        [C{:}]=OptionalBox.pair(); staticResult=[C{1},C{2}];",
        ))
        .unwrap();
    runtime.execute_entry(&[]).unwrap();
    array(&runtime, "result", &[11.0, 12.0]);
    array(&runtime, "staticResult", &[11.0, 22.0]);
}

#[test]
fn nested_index_selectors_execute_once_and_local_roots_stay_local() {
    let runtime = execute(
        "global calls; calls=0; result=f();\n\
        function y=f(); C={[1,2,3]}; C{index()}(end)=8; [C{index()}(1),C{index()}(end)]=pair(); y=C{1}; end\n\
        function x=index(); global calls; calls=calls+1; x=1; end\n\
        function [x,y]=pair(); x=11;y=22;end",
    );
    array(&runtime, "result", &[11.0, 2.0, 22.0]);
    scalar(&runtime, "calls", 3.0);
    assert!(runtime.workspace().get("C").is_none());
}

#[test]
fn indexed_optional_input_can_be_initialized_without_reading_it() {
    let runtime = execute("result=f(); function y=f(x); x(1)=9; y=x; end");
    // Indexed construction produces an array even for a scalar destination.
    array(&runtime, "result", &[9.0]);
}

#[test]
fn end_in_a_non_scalar_intermediate_pack_fails_before_rhs() {
    let runtime = execute(
        "global called; called=0; C={[1,2],[3,4]}; caught=0;\n\
        try; C{[1,2]}(end)=rhs(); catch err; caught=1; end;\n\
        function x=rhs(); global called; called=1; x=9; end",
    );
    scalar(&runtime, "called", 0.0);
    scalar(&runtime, "caught", 1.0);
}

#[test]
fn dynamically_requested_outputs_use_heap_continuations_on_a_small_native_stack() {
    let module = compile(
        "C={0,0}; [C{:}]=descend(600); result=[C{1},C{2}];\n\
        function [a,b]=descend(n); if n==0; a=1;b=2; else;\n\
        C={0,0}; [C{:}]=descend(n-1); a=C{1}+1; b=C{2}+2; end; end",
    );
    std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(move || {
            let mut runtime = Interpreter::new(module).unwrap();
            runtime.execute_entry(&[]).unwrap();
            array(&runtime, "result", &[601.0, 1202.0]);
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn scalar_and_bound_value_rhs_keep_their_non_call_assignment_semantics() {
    let runtime = execute(
        "C={0}; [C{1}]=7; a=C{1}; x=[2,3]; [C{1}]=x; b=C{1};\n\
        h=@(x) x+1; [C{1}]=h; saved=C{1}; c=saved(8); result=f(1);\n\
        function y=f(x); C={0}; [C{1}]=nargin; y=C{1}; end",
    );
    scalar(&runtime, "a", 7.0);
    array(&runtime, "b", &[2.0, 3.0]);
    scalar(&runtime, "c", 9.0);
    scalar(&runtime, "result", 1.0);
}

#[test]
fn bracketed_class_metadata_lookup_keeps_the_compiler_intrinsic() {
    let mut runtime = execute("classdef MetadataBox\n end");
    runtime.replace_module(compile("obj=MetadataBox(); C={0}; [C{1}]=metaclass(obj); metadata=C{1}; result=metadata.Sealed;")).unwrap();
    runtime.execute_entry(&[]).unwrap();
    assert_eq!(
        runtime.workspace().get("result"),
        Some(&Value::Logical(false))
    );
}
