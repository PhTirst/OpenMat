function slx_oracle(output_directory)
% OpenMat-authored miniature models; uses only public Simulink APIs.
% Generated SLX files and numerical observations stay outside public source.
if ~exist(output_directory, 'dir'), mkdir(output_directory); end
report = struct('schemaVersion', 1, 'release', version('-release'), 'ok', false);
try
    assert(strcmp(version('-release'), '2022b'), 'OpenMat:OracleRelease');
    make_feedback(output_directory, false);
    make_feedback(output_directory, true);
    make_counter(output_directory);
    make_sampled(output_directory);
    report.ok = true;
catch problem
    report.errorIdentifier = problem.identifier;
    if ~isempty(problem.stack)
        report.errorFunction = problem.stack(1).name;
        report.errorLine = problem.stack(1).line;
    end
end
write_json(fullfile(output_directory, 'oracle-status.json'), report);
if ~report.ok, error('OpenMat:OracleFailed', 'OpenMat oracle did not complete; inspect its normalized status.'); end
end

function make_feedback(output_directory, vector)
if vector, name = 'om_slx_vector'; else, name = 'om_slx_feedback'; end
model = new_system(name);
cleanup = onCleanup(@() close_system(model, 0));
set_param(model, 'SolverType', 'Fixed-step', 'Solver', 'ode4', ...
    'FixedStep', '0.05', 'StartTime', '0', 'StopTime', '1', ...
    'SignalLogging', 'on', 'SignalLoggingName', 'logsout', ...
    'ReturnWorkspaceOutputs', 'on');
add_block('simulink/Sources/Constant', [name '/one'], 'Position', [30 60 65 90]);
add_block('simulink/Math Operations/Sum', [name '/sum'], 'Inputs', '+-', 'Position', [115 60 140 90]);
add_block('simulink/Continuous/Integrator', [name '/state'], 'Position', [190 60 220 90]);
add_block('simulink/Math Operations/Gain', [name '/gain'], 'Gain', '1', 'Position', [160 155 200 185]);
add_block('simulink/Sinks/Scope', [name '/scope'], 'Position', [300 60 330 90]);
if vector
    set_param([name '/one'], 'Value', '[1 2]');
    set_param([name '/gain'], 'Gain', '[1 2]');
end
add_line(model, 'one/1', 'sum/1');
add_line(model, 'sum/1', 'state/1');
signal = add_line(model, 'state/1', 'scope/1');
add_line(model, 'state/1', 'gain/1');
add_line(model, 'gain/1', 'sum/2');
set_param(signal, 'Name', 'observed');
ports = get_param([name '/state'], 'PortHandles');
set_param(ports.Outport(1), 'DataLogging', 'on');
save_and_observe(model, name, output_directory);
end

function make_counter(output_directory)
name = 'om_slx_counter';
model = new_system(name);
cleanup = onCleanup(@() close_system(model, 0));
set_param(model, 'SolverType', 'Fixed-step', 'Solver', 'FixedStepDiscrete', ...
    'FixedStep', '0.1', 'StartTime', '0', 'StopTime', '0.3', ...
    'SignalLogging', 'on', 'SignalLoggingName', 'logsout', ...
    'ReturnWorkspaceOutputs', 'on');
add_block('simulink/Sources/Constant', [name '/one'], 'Value', '1');
add_block('simulink/Math Operations/Sum', [name '/sum'], 'Inputs', '++');
add_block('simulink/Discrete/Unit Delay', [name '/delay'], 'SampleTime', '0.1', 'InitialCondition', '0');
add_block('simulink/Sinks/Scope', [name '/scope']);
add_line(model, 'one/1', 'sum/1');
add_line(model, 'delay/1', 'sum/2');
add_line(model, 'sum/1', 'delay/1');
signal = add_line(model, 'delay/1', 'scope/1');
set_param(signal, 'Name', 'observed');
ports = get_param([name '/delay'], 'PortHandles');
set_param(ports.Outport(1), 'DataLogging', 'on');
save_and_observe(model, name, output_directory);
end

function save_and_observe(model, name, output_directory)
save_system(model, fullfile(output_directory, [name '.slx']));
result = sim(name);
signal = result.logsout.getElement('observed').Values;
observation = struct('schemaVersion', 1, 'model', name, ...
    'time', signal.Time, 'values', signal.Data);
write_json(fullfile(output_directory, [name '.oracle.json']), observation);
end

function make_sampled(output_directory)
name = 'om_slx_sampled';
model = new_system(name);
cleanup = onCleanup(@() close_system(model, 0));
set_param(model, 'SolverType', 'Fixed-step', 'Solver', 'ode4', ...
    'FixedStep', '0.05', 'StartTime', '0', 'StopTime', '0.4', ...
    'SignalLogging', 'on', 'SignalLoggingName', 'logsout', ...
    'ReturnWorkspaceOutputs', 'on');
add_block('simulink/Sources/Constant', [name '/one']);
add_block('simulink/Math Operations/Sum', [name '/sum'], 'Inputs', '+-');
add_block('simulink/Continuous/Integrator', [name '/state']);
add_block('simulink/Discrete/Unit Delay', [name '/delay'], 'SampleTime', '0.1');
add_block('simulink/Sinks/Scope', [name '/scope']);
add_line(model, 'one/1', 'sum/1');
add_line(model, 'delay/1', 'sum/2');
add_line(model, 'sum/1', 'state/1');
add_line(model, 'state/1', 'delay/1');
signal = add_line(model, 'state/1', 'scope/1');
set_param(signal, 'Name', 'observed');
ports = get_param([name '/state'], 'PortHandles');
set_param(ports.Outport(1), 'DataLogging', 'on');
save_and_observe(model, name, output_directory);
end

function write_json(path, value)
file = fopen(path, 'w', 'n', 'UTF-8');
assert(file >= 0, 'OpenMat:OracleOutput');
cleanup = onCleanup(@() fclose(file));
fprintf(file, '%s\n', jsonencode(value));
end
