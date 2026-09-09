fn argument_program(source: &str) {
    let mut engine = RuntimeEngine::new().unwrap();
    execute(&mut engine, source).unwrap_or_else(|error| panic!("{error:?}\n{source}"));
}

#[test]
fn arguments_name_value_syntax_works_without_validation_blocks() {
    argument_program(
        r#"
        [name,value,n]=f(Color=3);
        assert(isstring(name)); assert(name=="Color"); assert(value==3); assert(n==2);
        function [name,value,n]=f(varargin)
            name=varargin{1}; value=varargin{2}; n=nargin;
        end
    "#,
    );
}

#[test]
fn arguments_defaults_are_lazy_and_use_prior_converted_values() {
    argument_program(
        r"
        assert(isequal(f(2),[2 3 1])); assert(isequal(f(2,8),[2 8 2]));
        caught=false; try; f(); catch; caught=true; end; assert(caught);
        function y=f(x,z)
            arguments
                x (1,1) double {mustBePositive}
                z (1,1) double = x+1
            end
            y=[x z nargin];
        end
    ",
    );
}

#[test]
fn arguments_options_bind_case_prefix_duplicate_and_optional_positionals() {
    argument_program(
        r"
        assert(isequal(f(2,scale=4),[2 4 1]));
        assert(isequal(f('Sc',4),[10 4 0]));
        assert(isequal(f(2,'Scale',3,Scale=5),[2 5 1]));
        caught=false; try; f(1,Bogus=2); catch; caught=true; end; assert(caught);
        function y=f(x,opts)
            arguments
                x = 10
                opts.Scale (1,1) double = 2
                opts.Unset {mustBePositive}
            end
            assert(~isfield(opts,'Unset')); y=[x opts.Scale nargin];
        end
    ",
    );
}

#[test]
fn arguments_size_conversion_matches_scalar_expansion_and_vector_orientation() {
    argument_program(
        r"
        assert(isequal(f([1;2;3]),[1 2 3])); assert(isequal(f(7),[7 7 7]));
        caught=false; try; g([1 2 3]); catch; caught=true; end; assert(caught);
        function y=f(x)
            arguments
                x (1,3) double
            end
            y=x;
        end
        function y=g(x)
            arguments
                x (2,3) double
            end
            y=x;
        end
    ",
    );
}

#[test]
fn arguments_custom_validators_propagate_exceptions() {
    argument_program(
        r"
        assert(f(3)==3); caught=false;
        try; f(8); catch e; caught=strcmp(e.identifier,'Test:limit'); end; assert(caught);
        function y=f(x)
            arguments
                x {check_limit(x,5)}
            end
            y=x;
        end
        function check_limit(x,limit)
            if x>limit; error('Test:limit','above limit'); end
        end
    ",
    );
}

#[test]
fn arguments_output_blocks_run_on_explicit_and_implicit_returns() {
    argument_program(
        r"
        assert(isequal(f(0),[7 7 7])); assert(isequal(f(1),[8 8 8]));
        function y=f(x)
            arguments (Input)
                x
            end
            arguments (Output)
                y (1,3) double {mustBePositive}
            end
            y=7; if x==0; return; end; y=8;
        end
    ",
    );
}

#[test]
fn arguments_syntax_is_lossless_and_rejects_misplaced_blocks_and_named_arguments() {
    let source = "function y=f(x,o)\n% retained comment\narguments\nx (1,:) double {mustBeFinite}=1\no.Scale=2\nend\ny=x*o.Scale;\nend\nf(Scale=3);";
    let parsed = openmat_parser::parse(openmat_source::SourceId::new(1), source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    assert_eq!(
        parsed
            .syntax
            .tokens()
            .iter()
            .map(|token| token.text.as_str())
            .collect::<String>(),
        source
    );
    for source in ["x=f(X=1,2);", "x=f((X=1));", "x=C{X=1};", "x=f(a.b=1);"] {
        let parsed = openmat_parser::parse(openmat_source::SourceId::new(1), source);
        assert!(!parsed.diagnostics.is_empty(), "{source}");
    }
    for source in [
        "function f(x)\ny=1;\narguments\nx\nend\nend",
        "function f(x)\narguments\ny\nend\nend",
        "function f(x,y)\narguments\nx=1\ny\nend\nend",
        "function f(x)\narguments (Repeating)\nx=1\nend\nend",
        "function f(x)\narguments\nx Thing\nend\nimport pkg.*;\nend",
    ] {
        let mut engine = RuntimeEngine::new().unwrap();
        assert!(execute(&mut engine, source).is_err(), "{source}");
    }
}

#[test]
fn arguments_loaded_functions_and_class_methods_keep_binding_metadata() {
    let directory = TestDirectory::new("argument-blocks");
    fs::write(
        directory.path().join("scaled.m"),
        "function y=scaled(x,o)\narguments\nx double\no.Scale double=2\nend\ny=x*o.Scale;\nend",
    )
    .unwrap();
    fs::write(directory.path().join("ArgumentBox.m"), "classdef ArgumentBox\nmethods\nfunction y=scale(obj,x,o)\narguments\nobj (1,1) ArgumentBox\nx double\no.Factor=2\nend\ny=x*o.Factor;\nend\nend\nend").unwrap();
    let mut engine = RuntimeEngine::with_working_directory(directory.path()).unwrap();
    execute(
        &mut engine,
        "assert(scaled(3,Scale=4)==12); b=ArgumentBox(); assert(b.scale(4,Factor=3)==12);",
    )
    .unwrap();
    execute(
        &mut engine,
        "assert(scaled(5)==10); assert(b.scale(5)==10);",
    )
    .unwrap();
}

#[test]
fn arguments_defaults_capture_parent_scopes_and_skip_unused_expressions() {
    argument_program(
        r"
        assert(outer(7)==7); assert(lazy(3)==3);
        caught=false; try; lazy(); catch; caught=true; end; assert(caught);
        function y=outer(value)
            y=inner();
            function y=inner(x)
                arguments
                    x=value
                end
                y=x;
            end
        end
        function y=lazy(x)
            arguments
                x=error('Test:default','default evaluated')
            end
            y=x;
        end
    ",
    );
}

#[test]
fn arguments_common_validators_defaults_and_body_side_effects() {
    argument_program(
        r#"
        global entered; entered=0;
        assert(f(single(2))==2); assert(entered==1);
        caught=false; try; f(-1); catch; caught=true; end; assert(caught); assert(entered==1);
        caught=false; try; f(NaN); catch; caught=true; end; assert(caught); assert(entered==1);
        caught=false; try; f(); catch; caught=true; end; assert(caught); assert(entered==1);
        mustBeNonzero(1i); mustBeMember("a",["a","b"]); mustBeNumeric(2); mustBeInteger(uint64(3));
        function y=f(x)
            arguments
                x (1,1) double {mustBeReal,mustBeFinite,mustBeInteger,mustBePositive}=-1
            end
            global entered; entered=entered+1; y=x;
        end
    "#,
    );
}

#[test]
fn arguments_empty_vectors_case_sensitive_fields_and_output_arity() {
    argument_program(
        r"
        assert(isequal(size(row([])),[1 0]));
        o=options(Scale=3,scale=4); assert(o.Scale==3); assert(o.scale==4);
        caught=false; try; options('SCALE',3); catch; caught=true; end; assert(caught);
        caught=false; try; unused(); catch; caught=true; end; assert(caught);
        function y=row(x)
            arguments
                x (1,:) double
            end
            y=x;
        end
        function y=options(o)
            arguments
                o.Scale=1
                o.scale=2
            end
            y=o;
        end
        function [a,b]=unused()
            arguments (Output)
                b {mustBePositive}
            end
            a=1; b=-1;
        end
    ",
    );
}

#[test]
fn arguments_multiple_option_structs_function_handles_and_dynamic_outputs() {
    argument_program(
        r"
        h=@f; C=cell(1,2); [C{:}]=h(3,Right=4,Left=2);
        assert(C{1}==5); assert(C{2}==7);
        assert(feval(h,3,Left=5)==8);
        function [x,y]=f(v,a,b)
            arguments
                v
                a.Left=1
                b.Right=2
            end
            x=v+a.Left; y=v+b.Right;
        end
    ",
    );
}

#[test]
fn arguments_output_validation_is_outside_body_exception_handlers() {
    argument_program(
        r"
        caught=false; try; f(); catch; caught=true; end; assert(caught);
        function y=f()
            arguments (Output)
                y {mustBePositive}
            end
            try
                y=-1; return;
            catch
                y=2;
            end
        end
    ",
    );
}

#[test]
fn arguments_implicit_operations_do_not_use_same_named_input_variables() {
    argument_program(
        r"
        assert(f(1,2,3,4)==5);
        function y=f(isa,exist,isfield,double,x,o)
            arguments
                isa
                exist
                isfield
                double
                x (1,1) double=single(3)
                o.A=2
            end
            y=x+o.A;
        end
    ",
    );
}

#[test]
fn arguments_accept_scalar_function_handle_constraints() {
    argument_program(
        r"
        assert(f(@double)==3);
        function y=f(callback)
            arguments
                callback (1,1) function_handle {mustBeNonempty}
            end
            y=callback(single(3));
        end
    ",
    );
}

#[test]
fn arguments_class_conversion_can_load_a_source_constructor() {
    let directory = TestDirectory::new("argument-conversion");
    fs::write(directory.path().join("ArgumentValue.m"), "classdef ArgumentValue\nproperties\nValue=0\nend\nmethods\nfunction obj=ArgumentValue(x)\nobj.Value=x;\nend\nend\nend").unwrap();
    fs::write(
        directory.path().join("typed.m"),
        "function y=typed(x)\narguments\nx ArgumentValue\nend\ny=x.Value;\nend",
    )
    .unwrap();
    let mut engine = RuntimeEngine::with_working_directory(directory.path()).unwrap();
    execute(&mut engine, "assert(typed(3)==3);").unwrap();
}

#[test]
fn arguments_repeating_binds_cells_and_converts_each_element() {
    argument_program(
        r"
        [a,b,n]=f(single(1),[2;3],single(4),5);
        assert(isequal(a,{1,4})); assert(isequal(b,{[2,3],[5,5]})); assert(n==4);
        [a,b,n]=f(); assert(isequal(size(a),[1,0])); assert(isequal(size(b),[1,0])); assert(n==0);
        caught=false; try; f(1); catch; caught=true; end; assert(caught);
        function [a,b,n]=f(a,b)
            arguments (Input,Repeating)
                a (1,1) double {mustBePositive}
                b (1,2) double
            end
            n=nargin;
        end
    ",
    );
}

#[test]
fn arguments_repeating_validators_follow_group_order_and_prior_references() {
    argument_program(
        r"
        global validation_order; validation_order=[];
        [a,b]=f(single(1),2,single(3),4);
        assert(isequal(validation_order,[1,2,3,4]));
        assert(isequal(a,{1,3})); assert(isequal(b,{2,4}));
        function [a,b]=f(a,b)
            arguments (Repeating)
                a double {record(a)}
                b {pair(b,a)}
            end
        end
        function record(v)
            global validation_order; validation_order=[validation_order,v];
        end
        function pair(b,a)
            assert(isa(a,'double')); assert(b==a+1); record(b);
        end
    ",
    );
}

#[test]
fn arguments_repeating_options_start_only_at_group_boundaries() {
    argument_program(
        r#"
        [x,a,b,o,n]=f(9,1,2,Scale=3);
        assert(x==9); assert(isequal(a,{1})); assert(isequal(b,{2})); assert(o.Scale==3); assert(n==3);
        [x,a,b,o,n]=f(Scale=7); assert(x==8); assert(isempty(a)); assert(isempty(b)); assert(n==0);
        [x,a,b,o,n]=f(9,1,"Scale",2,3);
        assert(isequal(a,{1,2})); assert(isequal(b,{"Scale",3})); assert(o.Scale==2); assert(n==5);
        [x,a,b,o,n]=f(9,"Unknown",7); assert(isequal(a,{"Unknown"})); assert(n==3);
        [x,a,b,o,n]=f(9,Unknown=7); assert(isequal(a,{"Unknown"})); assert(n==3);
        [x,a,b,o,n]=f('S',3,4); assert(strcmp(x,'S')); assert(isequal(a,{3})); assert(isequal(b,{4})); assert(n==3);
        [x,a,b,o,n]=f(9,1,2,'Sca',3,Scale=4); assert(o.Scale==4);
        caught=false; try; f(9,1,Scale=3); catch; caught=true; end; assert(caught);
        caught=false; try; f(9,'S',3); catch; caught=true; end; assert(caught);
        function [x,a,b,o,n]=f(x,a,b,o)
            arguments
                x=8
            end
            arguments (Repeating)
                a
                b
            end
            arguments
                o.Scale=2
                o.Shape=1
            end
            n=nargin;
        end
    "#,
    );
}

#[test]
fn arguments_repeating_varargin_and_function_handles_preserve_call_counts() {
    argument_program(
        r"
        h=@f; [c,n]=feval(h,4,single(2),3); assert(isequal(c,{2,3})); assert(n==3);
        [c,n]=h(4); assert(isequal(size(c),[1,0])); assert(n==1);
        caught=false; try; h(); catch; caught=true; end; assert(caught);
        function [c,n]=f(required,varargin)
            arguments
                required
            end
            arguments (Repeating)
                varargin (1,1) double {mustBePositive}
            end
            c=varargin; n=nargin;
        end
    ",
    );
}

#[test]
fn arguments_repeating_outputs_support_named_tails_and_dynamic_requests() {
    argument_program(
        r"
        C=cell(1,3); [C{:}]=f(0);
        assert(C{1}==9); assert(isequal(C{2},[1,1])); assert(isequal(C{3},[2,2]));
        [a,b,c]=f(1); assert(a==9); assert(isequal(b,[1,1])); assert(isequal(c,[3,3]));
        [a,b]=g(); assert(a==4); assert(b==5);
        function [head,tail]=f(mode)
            arguments (Output)
                head double
            end
            arguments (Output,Repeating)
                tail (1,2) double {mustBePositive}
            end
            head=single(9); tail={single(1);2};
            if mode==0; return; end
            tail={1,2;3,4};
        end
        function varargout=g()
            arguments (Output,Repeating)
                varargout (1,1) double {mustBePositive}
            end
            varargout={single(4),5};
        end
    ",
    );
}

#[test]
fn arguments_repeating_output_errors_escape_body_catches_and_check_unused_values() {
    argument_program(
        r"
        empty();
        caught=false; try; a=empty(); catch; caught=true; end; assert(caught);
        caught=false; try; bad(0); catch; caught=true; end; assert(caught);
        caught=false; try; a=bad(0); catch; caught=true; end; assert(caught);
        caught=false; try; bad(1); catch; caught=true; end; assert(caught);
        function tail=empty()
            arguments (Output,Repeating)
                tail {mustBePositive}
            end
        end
        function tail=bad(mode)
            arguments (Output,Repeating)
                tail {mustBePositive}
            end
            try
                tail={1,-1};
                if mode==1; tail=7; end
                return;
            catch
                tail={2};
            end
        end
    ",
    );
}

#[test]
fn arguments_repeating_cells_remain_cells_in_nested_captures() {
    argument_program(
        r"
        assert(outer(2)==8);
        function y=outer(limit)
            y=inner(3,5);
            function y=inner(x)
                arguments (Repeating)
                    x {check(x,limit)}
                end
                y=readback();
                function y=readback()
                    y=x{1}+x{2};
                end
            end
        end
        function check(x,limit)
            assert(x>limit);
        end
    ",
    );
}

#[test]
fn arguments_repeating_rejects_malformed_layouts_before_execution() {
    for source in [
        "function f(x)\narguments (Repeating)\nx=1\nend\nend",
        "function f(x,y)\narguments (Repeating)\nx\nend\narguments (Repeating)\ny\nend\nend",
        "function f(x,y)\narguments (Repeating)\nx\nend\narguments\ny\nend\nend",
        "function f(x)\narguments (Repeating)\nx.A\nend\nend",
        "function f(x,varargin)\narguments (Repeating)\nx\nvarargin\nend\nend",
        "function f(x,y)\narguments (Repeating)\nx {check(x,y)}\ny\nend\nend",
        "function f(x,o)\narguments (Repeating)\nx {check(x,o)}\nend\narguments\no.Scale=1\nend\nend",
        "function [x,y]=f\narguments (Output,Repeating)\nx\ny\nend\nend",
        "function [x,y]=f\narguments (Output,Repeating)\nx\nend\nend",
        "function x=f\narguments (Output,Repeating)\nx=1\nend\nend",
        "function f\narguments (Repeating)\nend\nend",
    ] {
        let mut engine = RuntimeEngine::new().unwrap();
        assert!(execute(&mut engine, source).is_err(), "{source}");
    }
}

#[test]
fn arguments_repeating_source_loaded_functions_and_methods_keep_metadata() {
    let directory = TestDirectory::new("repeating-arguments");
    fs::write(directory.path().join("repeatcopy.m"), "function tail=repeatcopy(x)\narguments (Repeating)\nx double\nend\narguments (Output,Repeating)\ntail double\nend\ntail=x;\nend").unwrap();
    fs::write(directory.path().join("RepeatBox.m"), "classdef RepeatBox\nmethods\nfunction tail=copy(obj,x)\narguments\nobj RepeatBox\nend\narguments (Repeating)\nx double\nend\narguments (Output,Repeating)\ntail double\nend\ntail=x;\nend\nend\nend").unwrap();
    let mut engine = RuntimeEngine::with_working_directory(directory.path()).unwrap();
    execute(&mut engine, "[a,b]=repeatcopy(single(2),3); assert(a==2); assert(b==3); box=RepeatBox(); [a,b]=box.copy(4,5); assert(a==4); assert(b==5);").unwrap();
    execute(&mut engine, "[a,b]=repeatcopy(6,7); assert(a==6); assert(b==7); [a,b]=box.copy(8,9); assert(a==8); assert(b==9);").unwrap();
}

#[test]
fn arguments_range_validators_integrate_with_inputs_and_outputs() {
    argument_program(
        r#"
        assert(f(2)==2);
        caught=false; try; f(0); catch; caught=true; end; assert(caught);
        caught=false; try; f(5); catch; caught=true; end; assert(caught);
        mustBeGreaterThan([2,3],1); mustBeGreaterThanOrEqual(1,1);
        mustBeLessThan([2,3],4); mustBeLessThanOrEqual(4,4);
        mustBeInRange([0,1],0,1); mustBeInRange(.5,0,1,'exclude-lower','exclude-upper');
        mustBeInRange(0,0,1,"INC"); mustBeInRange([],3,1); mustBeInRange([],NaN,1);
        mustBeGreaterThan(sparse([1,2]),0); mustBeGreaterThanOrEqual(sparse([0,2]),0);
        caught=false; try; mustBeGreaterThan(sparse([0,2]),0); catch; caught=true; end; assert(caught);
        caught=false; try; mustBeInRange(0,0,1,'exclusive'); catch; caught=true; end; assert(caught);
        caught=false; try; mustBeInRange(1,0,1,'exclude-upper'); catch; caught=true; end; assert(caught);
        caught=false; try; mustBeInRange(.5,0,1,'inclusive','exclusive'); catch; caught=true; end; assert(caught);
        caught=false; try; mustBeGreaterThan([2,3],[0,1]); catch; caught=true; end; assert(caught);
        caught=false; try; mustBeInRange(NaN,0,1); catch; caught=true; end; assert(caught);
        function y=f(x)
            arguments
                x {mustBeGreaterThan(x,0),mustBeLessThanOrEqual(x,5)}
            end
            arguments (Output)
                y {mustBeInRange(y,0,4)}
            end
            y=x;
        end
    "#,
    );
}

#[test]
fn arguments_repeating_class_conversions_are_visible_to_later_validators() {
    let directory = TestDirectory::new("repeating-classes");
    fs::write(directory.path().join("RepeatValue.m"), "classdef RepeatValue\nproperties\nValue=0\nend\nmethods\nfunction obj=RepeatValue(x)\nobj.Value=x;\nend\nend\nend").unwrap();
    fs::write(directory.path().join("convertpairs.m"), "function y=convertpairs(x,z)\narguments (Repeating)\nx RepeatValue\nz {pair(z,x)}\nend\ny=x{1}.Value+x{2}.Value;\nend\nfunction pair(z,x)\nassert(isa(x,'RepeatValue')); assert(x.Value==z);\nend").unwrap();
    let mut engine = RuntimeEngine::with_working_directory(directory.path()).unwrap();
    execute(&mut engine, "assert(convertpairs(3,3,4,4)==7);").unwrap();
}

#[test]
fn arguments_repeating_documented_example_runs_end_to_end() {
    argument_program(
        r"
        [a,b] = pairs(1,2,3,4,Scale=2); assert(a==6); assert(b==14);
        function results=pairs(x,y,options)
            arguments (Repeating)
                x (1,:) double {mustBeFinite}
                y (1,:) double {mustBeGreaterThan(y,0)}
            end
            arguments
                options.Scale (1,1) double=1
            end
            arguments (Output,Repeating)
                results (1,:) double {mustBeFinite}
            end
            results=cell(1,numel(x));
            for k=1:numel(x)
                results{k}=(x{k}+y{k})*options.Scale;
            end
        end
    ",
    );
}
