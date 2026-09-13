function slx_hybrid_oracle(output_directory)
% Independently authored OpenMat models, built using public model APIs.
if ~exist(output_directory, 'dir'), mkdir(output_directory); end
report = struct('schemaVersion', 1, 'release', version('-release'), 'ok', false);
try
    assert(strcmp(version('-release'), '2022b'), 'OpenMat:OracleRelease');
    kinds = {'Saturate','Switch','RelationalOperator','Logic','Abs','MinMax','Integrator','DiscreteIntegrator'};
    model = begin_model('om_hybrid_defaults');
    for i = 1:numel(kinds)
        b = add(model, kinds{i}, kinds{i});
        names = fieldnames(get_param(b, 'DialogParameters')); values = struct();
        for j = 1:numel(names), values.(names{j}) = get_param(b, names{j}); end
        write_json(fullfile(output_directory, [kinds{i} '.defaults.json']), values);
    end
    close_system(model, 0);
    for kind = kinds(1:6), make_algebra(output_directory, kind{1}); end
    make_closed_loop(output_directory, false);
    make_closed_loop(output_directory, true);
    make_reduction(output_directory);
    for discrete = [false true]
        for mode = {'rising','falling','either'}
            make_reset(output_directory, discrete, mode{1});
            make_zero_edges(output_directory, discrete, mode{1});
        end
    end
    report.ok = true;
catch problem
    report.errorIdentifier = problem.identifier;
    report.causes = cellfun(@(e) e.identifier, problem.cause, 'UniformOutput', false);
    if ~isempty(problem.stack)
        report.errorFunction = problem.stack(1).name; report.errorLine = problem.stack(1).line;
    end
end
write_json(fullfile(output_directory, 'oracle-status.json'), report);
if ~report.ok, error('OpenMat:OracleFailed', 'OpenMat hybrid oracle failed; inspect normalized status.'); end
end

function make_closed_loop(directory, switched)
if switched, name='om_hybrid_switched_loop'; else, name='om_hybrid_saturated_pi'; end
model=begin_model(name); cleanup=onCleanup(@() close_system(model,0));
set_param(model,'StopTime','4');
add(model,'Constant','reference','Value','1'); add(model,'Sum','error','Inputs','+-');
add(model,'Integrator','plant'); add(model,'Sum','derivative','Inputs','+-');
add(model,'Gain','proportional','Gain','3');
add_line(model,'reference/1','error/1'); add_line(model,'plant/1','error/2');
add_line(model,'error/1','proportional/1');
if switched
    add(model,'Gain','slow','Gain','0.7'); add(model,'Step','mode','Time','2','Before','0','After','1');
    add(model,'Switch','actuator','Criteria','u2 >= Threshold','Threshold','0.5');
    add_line(model,'error/1','slow/1'); add_line(model,'proportional/1','actuator/1');
    add_line(model,'mode/1','actuator/2'); add_line(model,'slow/1','actuator/3');
else
    add(model,'Integrator','integral'); add(model,'Gain','ki','Gain','1.5');
    add(model,'Sum','raw','Inputs','++'); add(model,'Saturate','actuator','UpperLimit','0.8','LowerLimit','-0.8');
    add(model,'Sum','integral_input','Inputs','++'); add(model,'Sum','back_calculation','Inputs','+-');
    add_line(model,'error/1','ki/1'); add_line(model,'ki/1','integral_input/1');
    add_line(model,'actuator/1','back_calculation/1'); add_line(model,'raw/1','back_calculation/2');
    add_line(model,'back_calculation/1','integral_input/2'); add_line(model,'integral_input/1','integral/1');
    add_line(model,'integral/1','raw/2'); add_line(model,'proportional/1','raw/1'); add_line(model,'raw/1','actuator/1');
end
add_line(model,'actuator/1','derivative/1'); add_line(model,'plant/1','derivative/2');
add_line(model,'derivative/1','plant/1'); observe(model,name,directory,'plant');
end

function make_reduction(directory)
name='om_hybrid_vector_minmax'; model=begin_model(name); cleanup=onCleanup(@() close_system(model,0));
add(model,'Constant','vector','Value','[2 -3 1]'); add(model,'MinMax','minimum','Inputs','1','Function','min');
add_line(model,'vector/1','minimum/1'); observe(model,name,directory,'minimum');
end

function model = begin_model(name)
model = new_system(name);
set_param(model, 'SolverType','Fixed-step','Solver','ode4','FixedStep','0.01', ...
    'StartTime','0','StopTime','1','EnableMultiTasking','off','AutoInsertRateTranBlk','off', ...
    'SignalLogging','on','SignalLoggingName','logsout','ReturnWorkspaceOutputs','on');
end
function b = add(model, kind, name, varargin)
b = [get_param(model,'Name') '/' name]; add_block(['built-in/' kind], b, varargin{:});
end
function observe(model, name, directory, source)
add(model,'Scope','scope'); line = add_line(model,[source '/1'],'scope/1');
set_param(line,'Name','observed'); ports = get_param([name '/' source],'PortHandles');
set_param(ports.Outport(1),'DataLogging','on','DataLoggingNameMode','Custom','DataLoggingName','observed');
save_system(model,fullfile(directory,[name '.slx']));
result = sim(name); signal = result.logsout.getElement('observed').Values;
write_json(fullfile(directory,[name '.oracle.json']),struct('schemaVersion',1,'model',name, ...
    'time',signal.Time,'values',double(signal.Data)));
end
function make_algebra(directory, kind)
name = ['om_hybrid_' lower(kind)]; model = begin_model(name);
cleanup = onCleanup(@() close_system(model,0));
add(model,'Sin','source','Frequency','8','Phase','-0.4','SampleTime','0');
add(model,kind,'operation');
switch kind
    case 'Saturate'
        set_param([name '/operation'],'UpperLimit','0.6','LowerLimit','-0.25');
        add_line(model,'source/1','operation/1');
    case 'Switch'
        set_param([name '/operation'],'Threshold','0.3','Criteria','u2 >= Threshold');
        add(model,'Constant','high','Value','2'); add(model,'Constant','low','Value','-1');
        add_line(model,'high/1','operation/1'); add_line(model,'source/1','operation/2'); add_line(model,'low/1','operation/3');
    case {'RelationalOperator','Logic','MinMax'}
        add(model,'Constant','second','Value','0.25');
        if strcmp(kind,'RelationalOperator'), set_param([name '/operation'],'Operator','>='); end
        if strcmp(kind,'Logic'), set_param([name '/operation'],'Operator','XOR','Inputs','2'); end
        if strcmp(kind,'MinMax'), set_param([name '/operation'],'Inputs','2'); end
        if strcmp(kind,'Logic')
            add(model,'RelationalOperator','boolean_source','Operator','>=');
            add(model,'RelationalOperator','boolean_second','Operator','>=');
            add_line(model,'source/1','boolean_source/1'); add_line(model,'second/1','boolean_source/2');
            add_line(model,'second/1','boolean_second/1'); add_line(model,'second/1','boolean_second/2');
            add_line(model,'boolean_source/1','operation/1'); add_line(model,'boolean_second/1','operation/2');
        else
            add_line(model,'source/1','operation/1'); add_line(model,'second/1','operation/2');
        end
    otherwise
        add_line(model,'source/1','operation/1');
end
observe(model,name,directory,'operation');
end
function make_reset(directory, discrete, mode)
if discrete, kind = 'DiscreteIntegrator'; suffix = 'discrete'; rate = '0.05';
else, kind = 'Integrator'; suffix = 'continuous'; rate = '0'; end
name = ['om_hybrid_reset_' suffix '_' mode]; model = begin_model(name);
cleanup = onCleanup(@() close_system(model,0));
add(model,'Sin','trigger','Frequency','8','Phase','-0.4','SampleTime',rate);
add(model,'Constant','drive','Value','2');
target = add(model,kind,'state','InitialCondition','0.75','ExternalReset',mode);
if discrete, set_param(target,'SampleTime',rate,'IntegratorMethod','Integration: Forward Euler'); end
add_line(model,'drive/1','state/1'); add_line(model,'trigger/1','state/2');
observe(model,name,directory,'state');
end
function write_json(path, value)
file = fopen(path,'w','n','UTF-8'); assert(file >= 0,'OpenMat:OracleOutput');
cleanup = onCleanup(@() fclose(file)); fprintf(file,'%s\n',jsonencode(value));
end

function make_zero_edges(directory, discrete, mode)
if discrete, kind='DiscreteIntegrator'; suffix='discrete'; rate='0.05';
else, kind='Integrator'; suffix='continuous'; rate='0'; end
name=['om_hybrid_zero_' suffix '_' mode]; model=begin_model(name);
cleanup=onCleanup(@() close_system(model,0));
set_param(model,'StopTime','0.4');
add(model,'Constant','drive','Value','1'); add(model,'Constant','base','Value','-1');
add(model,'Sum','trigger','Inputs','+++++++'); add_line(model,'base/1','trigger/1');
steps=[1 1 -1 -1 1 1];
for i=1:6
    id=['edge' num2str(i)]; add(model,'Step',id,'Time',num2str(i*0.05,17),'Before','0','After',num2str(steps(i)),'SampleTime',rate);
    add_line(model,[id '/1'],['trigger/' num2str(i+1)]);
end
target=add(model,kind,'state','InitialCondition','0.75','ExternalReset',mode);
if discrete, set_param(target,'SampleTime',rate,'IntegratorMethod','Integration: Forward Euler'); end
add_line(model,'drive/1','state/1'); add_line(model,'trigger/1','state/2');
observe(model,name,directory,'state');
end
