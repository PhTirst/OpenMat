function slx_conditional_oracle(output_directory)
% OpenMat-authored models built only through public model APIs.
if ~exist(output_directory,'dir'), mkdir(output_directory); end
report=struct('schemaVersion',1,'release',version('-release'),'ok',false);
try
    assert(strcmp(version('-release'),'2022b'),'OpenMat:OracleRelease');
    for state={'held','reset'}
        for output={'held','reset'}
            build_case(output_directory,'enabled',state{1},output{1},'rising',-1);
            build_case(output_directory,'enabled',state{1},output{1},'rising',1);
            build_case(output_directory,'enabled',state{1},output{1},'rising',-1,'DiscreteIntegrator');
            build_case(output_directory,'enabled',state{1},output{1},'rising',1,'DiscreteIntegrator');
        end
    end
    for edge={'rising','falling','either'}
        for initial=[-1 0 1]
            build_case(output_directory,'triggered','held','held',edge{1},initial);
        end
    end
    report.ok=true;
catch problem
    report.errorIdentifier=problem.identifier;
    report.causes=cellfun(@(e)e.identifier,problem.cause,'UniformOutput',false);
    if ~isempty(problem.stack)
        report.errorFunction=problem.stack(1).name;
        report.errorLine=problem.stack(1).line;
    end
end
write_json(fullfile(output_directory,'oracle-status.json'),report);
if ~report.ok, error('OpenMat:OracleFailed','OpenMat conditional oracle failed.'); end
end

function build_case(directory,kind,state,output,edge,initial,memory)
if nargin<7, memory='UnitDelay'; end
name=sprintf('om_cond_%s_%s_%s_%s_%d',kind,state,output,edge,initial+1);
if ~strcmp(memory,'UnitDelay'), name=[name '_integral']; end
new_system(name); cleanup=onCleanup(@()close_system(name,0));
set_param(name,'SolverType','Fixed-step','Solver','ode4','FixedStep','0.01', ...
    'StartTime','0','StopTime','0.9','EnableMultiTasking','off','AutoInsertRateTranBlk','off', ...
    'SignalLogging','on','SignalLoggingName','logsout','ReturnWorkspaceOutputs','on');
levels=[initial 0 0 1 1 0 0 -1 0 1];
add_block('built-in/Constant',[name '/base'],'Value',num2str(initial));
add_block('built-in/Sum',[name '/control'],'Inputs',repmat('+',1,numel(levels)));
add_line(name,'base/1','control/1');
for i=2:numel(levels)
    b=sprintf('step%d',i);
    add_block('built-in/Step',[name '/' b],'Time',num2str((i-1)*0.1,17), ...
        'Before','0','After',num2str(levels(i)-levels(i-1)),'SampleTime','0.1');
    add_line(name,[b '/1'],sprintf('control/%d',i));
end
group=[name '/counter']; add_block('built-in/SubSystem',group);
if strcmp(kind,'enabled')
    add_block('built-in/EnablePort',[group '/Enable'],'StatesWhenEnabling',state);
    rate='0.1';
else
    add_block('built-in/TriggerPort',[group '/Trigger'],'TriggerType',edge);
    rate='-1';
end
add_block('built-in/Constant',[group '/one'],'Value','1');
add_block(['built-in/' memory],[group '/memory'],'InitialCondition','3','SampleTime',rate);
add_block('built-in/Outport',[group '/out'],'Port','1','InitialOutput','-7','OutputWhenDisabled',output);
if strcmp(memory,'UnitDelay')
    add_block('built-in/Sum',[group '/increment'],'Inputs','++');
    add_line(group,'memory/1','increment/1'); add_line(group,'one/1','increment/2');
    add_line(group,'increment/1','memory/1');
else
    set_param([group '/memory'],'gainval','5','IntegratorMethod','Integration: Forward Euler');
    add_line(group,'one/1','memory/1');
end
add_line(group,'memory/1','out/1');
source=get_param([name '/control'],'PortHandles'); target=get_param(group,'PortHandles');
if strcmp(kind,'enabled'), port=target.Enable; else, port=target.Trigger; end
add_line(name,source.Outport(1),port);
add_block('built-in/ZeroOrderHold',[name '/sample'],'SampleTime','0.1');
add_block('built-in/Scope',[name '/scope']);
add_line(name,'counter/1','sample/1'); add_line(name,'sample/1','scope/1');
ports=get_param([name '/sample'],'PortHandles');
set_param(ports.Outport(1),'DataLogging','on','DataLoggingNameMode','Custom','DataLoggingName','observed');
save_system(name,fullfile(directory,[name '.slx']));
result=sim(name); signal=result.logsout.getElement('observed').Values;
write_json(fullfile(directory,[name '.oracle.json']),struct('schemaVersion',1,'model',name, ...
    'time',signal.Time,'values',double(signal.Data)));
end

function write_json(path,value)
file=fopen(path,'w','n','UTF-8'); assert(file>=0,'OpenMat:OracleOutput');
cleanup=onCleanup(@()fclose(file)); fprintf(file,'%s\n',jsonencode(value));
end
