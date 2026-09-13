function block_editor_oracle(outputDirectory)
% Independently authored interface/value probe; no vendor fixtures are copied.
if ~exist(outputDirectory, 'dir'), mkdir(outputDirectory); end
model = 'om_block_editor_probe';
new_system(model);
cleanup = onCleanup(@() close_system(model, 0));
load_system('simulink');
types = {'Constant','Gain','Sum','Product','Integrator','DiscreteIntegrator', ...
    'UnitDelay','TransferFcn','StateSpace','Inport','Outport','EnablePort','TriggerPort'};
libraryPaths = {'Sources/Constant','Math Operations/Gain','Math Operations/Sum', ...
    'Math Operations/Product','Continuous/Integrator','Discrete/Discrete-Time Integrator', ...
    'Discrete/Unit Delay','Continuous/Transfer Fcn','Continuous/State-Space', ...
    'Ports & Subsystems/In1','Ports & Subsystems/Out1', ...
    'Ports & Subsystems/Enable','Ports & Subsystems/Trigger'};
names = {'Value','Gain','Multiplication','Inputs','InitialCondition', ...
    'SampleTime','ExternalReset','LimitOutput','InitialConditionSource', ...
    'IntegratorMethod','Numerator','Denominator','A','B','C','D','X0', ...
    'Port','StatesWhenEnabling','TriggerType','OutputWhenDisabled','InitialOutput'};
metadata = struct();
for i = 1:numel(types)
    if strcmp(types{i}, 'EnablePort') || strcmp(types{i}, 'TriggerPort')
        parent = [model '/' types{i} '_system'];
        add_block('built-in/SubSystem', parent);
        path = [parent '/' types{i}];
    else
        path = [model '/' types{i}];
    end
    add_block(['simulink/' libraryPaths{i}], path);
    parameters = get_param(path, 'DialogParameters');
    record = struct();
    for j = 1:numel(names)
        if isfield(parameters, names{j})
            record.(names{j}) = get_param(path, names{j});
        end
    end
    metadata.(types{i}) = record;
end
K = 2; Ts = 0.05;
values = {K, 2*pi, [K 3*K], [1;2], eye(2)*K, [1 Ts], sin(pi/2), zeros(1,3)};
expressions = {'K','2*pi','[K 3*K]','[1;2]','eye(2)*K','[1 Ts]','sin(pi/2)','zeros(1,3)'};
cases = cell(1,numel(values));
for i = 1:numel(values)
    v = values{i};
    cases{i} = struct('expression',expressions{i},'rows',size(v,1), ...
        'columns',size(v,2),'values',v(:)');
end
result = struct('schemaVersion',1,'release',version('-release'), ...
    'defaults',metadata,'source','K = 2; Ts = 0.05;','cases',{cases});
f = fopen(fullfile(outputDirectory,'block-editor.json'), 'w', 'n', 'UTF-8');
assert(f >= 0);
fileCleanup = onCleanup(@() fclose(f));
fwrite(f,jsonencode(result),'char');
end
