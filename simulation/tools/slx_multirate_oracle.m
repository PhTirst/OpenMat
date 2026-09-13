function slx_multirate_oracle(output_directory)
% OpenMat-authored fixtures; only public model construction/observation APIs.
if ~exist(output_directory, 'dir'), mkdir(output_directory); end
report = struct('schemaVersion', 1, 'release', version('-release'), 'ok', false);
try
    assert(strcmp(version('-release'), '2022b'), 'OpenMat:OracleRelease');
    probe_defaults(output_directory);
    for discrete = [false true]
        make_inherited(output_directory, discrete);
    end
    make_hold(output_directory);
    make_discrete(output_directory, 'DiscreteStateSpace');
    make_discrete(output_directory, 'DiscreteTransferFcn');
    make_discrete(output_directory, 'DiscreteIntegrator');
    for fast = [false true]
        for deterministic = [false true]
            make_transition(output_directory, fast, deterministic);
        end
    end
    for observed = {'plant', 'control', 'monitor'}
        make_closed_loop(output_directory, observed{1});
    end
    make_inherited_filter(output_directory);
    report.ok = true;
catch problem
    report.errorIdentifier = problem.identifier;
    if ~isempty(problem.stack)
        report.errorFunction = problem.stack(1).name;
        report.errorLine = problem.stack(1).line;
    end
end
write_json(fullfile(output_directory, 'oracle-status.json'), report);
if ~report.ok, error('OpenMat:OracleFailed', 'OpenMat multirate oracle failed; inspect normalized status.'); end
end

function probe_defaults(output_directory)
name = 'om_multirate_defaults'; model = begin_model(name);
cleanup = onCleanup(@() close_system(model, 0));
for kind = {'DiscreteStateSpace', 'DiscreteTransferFcn', 'DiscreteIntegrator', 'ZeroOrderHold', 'RateTransition'}
    target = add(model, kind{1}, kind{1});
    parameters = get_param(target, 'DialogParameters'); names = fieldnames(parameters); values = struct();
    for index = 1:numel(names), values.(names{index}) = get_param(target, names{index}); end
    write_json(fullfile(output_directory, [kind{1} '.defaults.json']), values);
end
end

function model = begin_model(name)
model = new_system(name);
set_param(model, 'SolverType', 'Fixed-step', 'Solver', 'ode4', ...
    'FixedStep', '0.01', 'StartTime', '0', 'StopTime', '0.5', ...
    'EnableMultiTasking', 'off', 'AutoInsertRateTranBlk', 'off', ...
    'SignalLogging', 'on', 'SignalLoggingName', 'logsout', ...
    'ReturnWorkspaceOutputs', 'on');
end

function block = add(model, kind, name, varargin)
block = [get_param(model, 'Name') '/' name];
add_block(['built-in/' kind], block, varargin{:});
end

function observe(model, name, output_directory, source)
add(model, 'Scope', 'scope');
line = add_line(model, [source '/1'], 'scope/1');
set_param(line, 'Name', 'observed');
ports = get_param([name '/' source], 'PortHandles');
set_param(ports.Outport(1), 'DataLogging', 'on', 'DataLoggingNameMode', 'Custom', 'DataLoggingName', 'observed');
save_system(model, fullfile(output_directory, [name '.slx']));
result = sim(name);
signal = result.logsout.getElement('observed').Values;
write_json(fullfile(output_directory, [name '.oracle.json']), ...
    struct('schemaVersion', 1, 'model', name, 'time', signal.Time, 'values', signal.Data));
feval(name, [], [], [], 'compile');
cleanup = onCleanup(@() feval(name, [], [], [], 'term'));
blocks = find_system(name, 'SearchDepth', 1, 'Type', 'Block');
rates = struct();
for index = 1:numel(blocks)
    rates.(get_param(blocks{index}, 'Name')) = get_param(blocks{index}, 'CompiledSampleTime');
end
write_json(fullfile(output_directory, [name '.rates.json']), rates);
end

function make_inherited(output_directory, discrete)
if discrete, suffix = 'discrete'; else, suffix = 'continuous'; end
name = ['om_multirate_inherited_' suffix];
model = begin_model(name); cleanup = onCleanup(@() close_system(model, 0));
add(model, 'Sin', 'source', 'Amplitude', '2', 'Frequency', '3', 'SampleTime', '-1');
add(model, 'Gain', 'gain', 'Gain', '0.5');
if discrete
    add(model, 'UnitDelay', 'state', 'SampleTime', '0.1', 'InitialCondition', '-1');
else
    add(model, 'Integrator', 'state', 'InitialCondition', '-1');
end
add_line(model, 'source/1', 'gain/1'); add_line(model, 'gain/1', 'state/1');
observe(model, name, output_directory, 'state');
end

function make_hold(output_directory)
name = 'om_multirate_hold'; model = begin_model(name);
cleanup = onCleanup(@() close_system(model, 0));
add(model, 'Sin', 'source', 'Frequency', '2', 'SampleTime', '0');
add(model, 'ZeroOrderHold', 'sample', 'SampleTime', '0.1');
add(model, 'Integrator', 'state', 'InitialCondition', '0');
add_line(model, 'source/1', 'sample/1'); add_line(model, 'sample/1', 'state/1');
observe(model, name, output_directory, 'state');
end

function make_discrete(output_directory, kind)
name = ['om_multirate_' lower(kind)]; model = begin_model(name);
cleanup = onCleanup(@() close_system(model, 0));
add(model, 'Sin', 'source', 'SampleTime', '0.05', 'Frequency', '2');
if strcmp(kind, 'DiscreteStateSpace')
    target = add(model, kind, 'state', 'SampleTime', '0.05', ...
        'A', '[0.8 0.1; 0 0.7]', 'B', '[1; 2]', 'C', '[1 -1]', 'D', '0.25', 'InitialCondition', '[1; -1]');
elseif strcmp(kind, 'DiscreteTransferFcn')
    target = add(model, kind, 'state', 'SampleTime', '0.05', ...
        'Numerator', '[0.5 0.25]', 'Denominator', '[1 -0.5]', 'InitialStates', '0.75');
else
    target = add(model, kind, 'state', 'SampleTime', '0.05', ...
        'IntegratorMethod', 'Integration: Forward Euler', 'InitialCondition', '0.75');
end
parameters = get_param(target, 'DialogParameters');
names = fieldnames(parameters); values = struct();
for index = 1:numel(names), values.(names{index}) = get_param(target, names{index}); end
write_json(fullfile(output_directory, [name '.parameters.json']), values);
add_line(model, 'source/1', 'state/1');
observe(model, name, output_directory, 'state');
end

function make_transition(output_directory, fast, deterministic)
if fast, direction = 'fast_to_slow'; input_rate = '0.02'; output_rate = '0.1';
else, direction = 'slow_to_fast'; input_rate = '0.1'; output_rate = '0.02'; end
if deterministic, mode = 'on'; else, mode = 'off'; end
name = ['om_multirate_rt_' direction '_' mode]; model = begin_model(name);
cleanup = onCleanup(@() close_system(model, 0));
add(model, 'Sin', 'source', 'SampleTime', input_rate, 'Frequency', '2', 'Phase', '0.3');
target = add(model, 'RateTransition', 'transfer', 'OutPortSampleTimeOpt', 'Specify', ...
    'OutPortSampleTime', output_rate, 'Integrity', mode, 'Deterministic', mode, 'InitialCondition', '-2');
parameters = get_param(target, 'DialogParameters');
names = fieldnames(parameters); values = struct();
for index = 1:numel(names), values.(names{index}) = get_param(target, names{index}); end
write_json(fullfile(output_directory, [name '.parameters.json']), values);
add_line(model, 'source/1', 'transfer/1');
observe(model, name, output_directory, 'transfer');
end

function make_closed_loop(output_directory, source)
name = ['om_multirate_closed_loop_' source]; model = begin_model(name);
cleanup = onCleanup(@() close_system(model, 0));
set_param(model, 'FixedStep', '0.001', 'StopTime', '1');
add(model, 'Step', 'reference', 'Time', '0.03', 'SampleTime', '0');
add(model, 'Sum', 'error', 'Inputs', '+-');
add(model, 'ZeroOrderHold', 'sample', 'SampleTime', '0.01');
add(model, 'Gain', 'proportional', 'Gain', '2');
add(model, 'DiscreteIntegrator', 'integral', 'SampleTime', '0.01', 'gainval', '3', 'IntegratorMethod', 'Integration: Forward Euler');
add(model, 'Sum', 'control', 'Inputs', '++');
add(model, 'StateSpace', 'plant', 'A', '-2', 'B', '1', 'C', '1', 'D', '0');
add(model, 'RateTransition', 'monitor', 'OutPortSampleTimeOpt', 'Specify', 'OutPortSampleTime', '0.1', 'Integrity', 'on', 'Deterministic', 'on');
add_line(model, 'reference/1', 'error/1');
add_line(model, 'plant/1', 'sample/1'); add_line(model, 'sample/1', 'error/2');
add_line(model, 'error/1', 'proportional/1'); add_line(model, 'error/1', 'integral/1');
add_line(model, 'proportional/1', 'control/1'); add_line(model, 'integral/1', 'control/2');
add_line(model, 'control/1', 'plant/1'); add_line(model, 'sample/1', 'monitor/1');
% Sample the reference at the controller rate so the whole controller is discrete.
set_param([name '/reference'], 'SampleTime', '0.01');
observe(model, name, output_directory, source);
end

function make_inherited_filter(output_directory)
name = 'om_multirate_inherited_filter'; model = begin_model(name);
cleanup = onCleanup(@() close_system(model, 0));
add(model, 'Sin', 'source', 'SampleTime', '0.05', 'Frequency', '2');
add(model, 'DiscreteTransferFcn', 'state');
add_line(model, 'source/1', 'state/1');
observe(model, name, output_directory, 'state');
end

function write_json(path, value)
file = fopen(path, 'w', 'n', 'UTF-8'); assert(file >= 0, 'OpenMat:OracleOutput');
cleanup = onCleanup(@() fclose(file)); fprintf(file, '%s\n', jsonencode(value));
end
