function slx_control_oracle(output_directory)
% OpenMat-authored control models, constructed exclusively with public APIs.
if ~exist(output_directory, 'dir'), mkdir(output_directory); end
report = struct('schemaVersion', 1, 'release', version('-release'), 'ok', false);
try
    assert(strcmp(version('-release'), '2022b'), 'OpenMat:OracleRelease');
    parameters = sprintf('K = 2;\nA = [-1 1; 0 -2];\nB = [1 0; 0 1];\nC = [1 0; 0 1];\nD = [0 0; 0 0];\nx0 = [1; -1];\nTs = 0.05;\n');
    file = fopen(fullfile(output_directory, 'parameters.m'), 'w', 'n', 'UTF-8');
    assert(file >= 0, 'OpenMat:OracleOutput');
    fprintf(file, '%s', parameters); fclose(file);
    evalin('base', parameters);
    make_nested(output_directory);
    make_state_space(output_directory, false);
    make_state_space(output_directory, true);
    make_transfer(output_directory, false);
    make_transfer(output_directory, true);
    make_math(output_directory);
    make_routing(output_directory);
    make_step(output_directory, 0);
    make_step(output_directory, 0.15);
    make_step(output_directory, 0.2);
    make_step(output_directory, 0.3);
    make_step(output_directory, 0.6);
    make_defaults(output_directory, 'StateSpace');
    make_defaults(output_directory, 'TransferFcn');
    make_defaults(output_directory, 'Bias');
    make_defaults(output_directory, 'Sin');
    make_defaults(output_directory, 'Step');
    make_gain_transfer(output_directory);
    report.ok = true;
catch problem
    report.errorIdentifier = problem.identifier;
    if ~isempty(problem.stack)
        report.errorFunction = problem.stack(1).name;
        report.errorLine = problem.stack(1).line;
    end
end
write_json(fullfile(output_directory, 'oracle-status.json'), report);
if ~report.ok, error('OpenMat:OracleFailed', 'OpenMat control oracle failed; inspect normalized status.'); end
end

function model = begin_model(name, step, stop)
model = new_system(name);
set_param(model, 'SolverType', 'Fixed-step', 'Solver', 'ode4', ...
    'FixedStep', step, 'StartTime', '0', 'StopTime', stop, ...
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
set_param(ports.Outport(1), 'DataLogging', 'on');
save_system(model, fullfile(output_directory, [name '.slx']));
result = sim(name);
signal = result.logsout.getElement('observed').Values;
write_json(fullfile(output_directory, [name '.oracle.json']), ...
    struct('schemaVersion', 1, 'model', name, 'time', signal.Time, 'values', signal.Data));
end

function make_nested(output_directory)
name = 'om_control_nested'; model = begin_model(name, 'Ts', '1');
cleanup = onCleanup(@() close_system(model, 0));
add(model, 'Constant', 'one', 'Value', '1');
add(model, 'Sum', 'sum', 'Inputs', '+-');
plant = add(model, 'SubSystem', 'plant', 'TreatAsAtomicUnit', 'off');
add_block('built-in/Inport', [plant '/u']);
inner = [plant '/inner']; add_block('built-in/SubSystem', inner, 'TreatAsAtomicUnit', 'off');
add_block('built-in/Inport', [inner '/u']);
add_block('built-in/Integrator', [inner '/state'], 'InitialCondition', '0');
add_block('built-in/Outport', [inner '/y']);
add_line(inner, 'u/1', 'state/1'); add_line(inner, 'state/1', 'y/1');
add_block('built-in/Outport', [plant '/y']);
add_line(plant, 'u/1', 'inner/1'); add_line(plant, 'inner/1', 'y/1');
add(model, 'Gain', 'gain', 'Gain', 'K');
add_line(model, 'one/1', 'sum/1'); add_line(model, 'gain/1', 'sum/2');
add_line(model, 'sum/1', 'plant/1'); add_line(model, 'plant/1', 'gain/1');
observe(model, name, output_directory, 'plant');
end

function make_state_space(output_directory, feedthrough)
if feedthrough, name = 'om_control_ss_direct'; else, name = 'om_control_ss'; end
model = begin_model(name, 'Ts', '1'); cleanup = onCleanup(@() close_system(model, 0));
add(model, 'Constant', 'input', 'Value', '[1 2]');
if feedthrough, direct = '[0.25 0; 0 -0.5]'; else, direct = 'D'; end
add(model, 'StateSpace', 'plant', 'A', 'A', 'B', 'B', 'C', 'C', 'D', direct, 'X0', 'x0');
add_line(model, 'input/1', 'plant/1');
observe(model, name, output_directory, 'plant');
end

function make_transfer(output_directory, feedthrough)
if feedthrough, name = 'om_control_tf_direct'; numerator = '[1 3 2]';
else, name = 'om_control_tf'; numerator = '[1 2]'; end
model = begin_model(name, '0.05', '1'); cleanup = onCleanup(@() close_system(model, 0));
add(model, 'Constant', 'input', 'Value', 'K');
add(model, 'TransferFcn', 'plant', 'Numerator', numerator, 'Denominator', '[2 6 4]');
add_line(model, 'input/1', 'plant/1');
observe(model, name, output_directory, 'plant');
end

function make_math(output_directory)
name = 'om_control_math'; model = begin_model(name, '0.05', '1');
cleanup = onCleanup(@() close_system(model, 0));
add(model, 'Sin', 'sine', 'SineType', 'Time based', 'TimeSource', 'Use simulation time', ...
    'Amplitude', 'K', 'Bias', '0.2', 'Frequency', '2*pi', 'Phase', 'pi/4', 'SampleTime', '0');
add(model, 'Bias', 'bias', 'Bias', '0.5');
add(model, 'Constant', 'scale', 'Value', '[1 2]');
add(model, 'Product', 'product', 'Inputs', '**', 'Multiplication', 'Element-wise(.*)');
add_line(model, 'sine/1', 'bias/1'); add_line(model, 'bias/1', 'product/1');
add_line(model, 'scale/1', 'product/2'); observe(model, name, output_directory, 'product');
end

function make_routing(output_directory)
name = 'om_control_routing'; model = begin_model(name, '0.05', '0.2');
cleanup = onCleanup(@() close_system(model, 0));
add(model, 'Constant', 'input', 'Value', '[1 2 3]');
add(model, 'Demux', 'split', 'Outputs', '[2 1]');
add(model, 'Ground', 'ground'); add(model, 'Terminator', 'unused');
add(model, 'Mux', 'join', 'Inputs', '2');
add_line(model, 'input/1', 'split/1'); add_line(model, 'split/1', 'join/1');
add_line(model, 'split/2', 'unused/1'); add_line(model, 'ground/1', 'join/2');
observe(model, name, output_directory, 'join');
end

function make_step(output_directory, event_time)
name = sprintf('om_control_step_%03d', round(event_time * 100));
model = begin_model(name, '0.1', '1'); cleanup = onCleanup(@() close_system(model, 0));
add(model, 'Step', 'input', 'Time', num2str(event_time, 17), 'Before', '0', 'After', '1', 'SampleTime', '0');
add(model, 'Integrator', 'state'); add(model, 'Mux', 'join', 'Inputs', '2');
add_line(model, 'input/1', 'state/1'); add_line(model, 'input/1', 'join/1');
add_line(model, 'state/1', 'join/2'); observe(model, name, output_directory, 'join');
end

function make_defaults(output_directory, kind)
name = ['om_control_default_' lower(kind)];
model = begin_model(name, '0.1', '1.2'); cleanup = onCleanup(@() close_system(model, 0));
plant = add(model, kind, 'plant');
keys = {};
switch kind
    case 'StateSpace', keys = {'A','B','C','D','InitialCondition'};
    case 'TransferFcn', keys = {'Numerator','Denominator'};
    case 'Bias', keys = {'Bias'};
    case 'Sin', keys = {'Amplitude','Bias','Frequency','Phase','SampleTime'};
    case 'Step', keys = {'Time','Before','After','SampleTime'};
end
defaults = struct();
for index = 1:numel(keys), defaults.(keys{index}) = get_param(plant, keys{index}); end
write_json(fullfile(output_directory, [name '.parameters.json']), defaults);
if ~any(strcmp(kind, {'Sin', 'Step'}))
    add(model, 'Constant', 'input', 'Value', '2'); add_line(model, 'input/1', 'plant/1');
end
observe(model, name, output_directory, 'plant');
end

function make_gain_transfer(output_directory)
name = 'om_control_tf_gain'; model = begin_model(name, '0.1', '1');
cleanup = onCleanup(@() close_system(model, 0));
add(model, 'Sin', 'input', 'SampleTime', '0');
add(model, 'TransferFcn', 'plant', 'Numerator', '3', 'Denominator', '2');
add_line(model, 'input/1', 'plant/1'); observe(model, name, output_directory, 'plant');
end

function write_json(path, value)
file = fopen(path, 'w', 'n', 'UTF-8'); assert(file >= 0, 'OpenMat:OracleOutput');
cleanup = onCleanup(@() fclose(file)); fprintf(file, '%s\n', jsonencode(value));
end
