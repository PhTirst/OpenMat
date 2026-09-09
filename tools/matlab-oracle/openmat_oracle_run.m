function openmat_oracle_run(config_path)
%OPENMAT_ORACLE_RUN Execute OpenMat-authored probes and write normalized JSON.

config = jsondecode(fileread(config_path));
if ~isfolder(config.output_directory)
    mkdir(config.output_directory);
end

cases = config.cases;
for index = 1:numel(cases)
    observation = execute_case(cases(index), config);
    [observation, json_text] = encode_bounded_observation(observation);
    output_path = fullfile(config.output_directory, ...
        [char(cases(index).id), '.json']);
    write_json(output_path, json_text);
end
end

function observation = execute_case(case_config, config)
observation = struct();
schema_version = double(case_config.schema_version);
if schema_version ~= 1 && schema_version ~= 2
    error('OpenMatOracle:UnsupportedSchemaVersion', ...
        'Unsupported conformance schema version.');
end
observation.schema_version = schema_version;
observation.case_id = char(case_config.id);
observation.oracle = struct('name', 'MATLAB', ...
    'release', ['R', version('-release')]);

old_path = path;
path_cleanup = onCleanup(@() path(old_path)); %#ok<NASGU>
addpath(config.program_directory);
addpath(config.support_directory);

try
    run(char(case_config.source_path));
    if ~exist('openmat_result', 'var')
        error('OpenMatOracle:MissingResult', ...
            'Probe did not assign the required result variable.');
    end
    observation.outcome = 'ok';
    observation.value = normalize_value(openmat_result, schema_version);
catch exception
    observation.outcome = 'error';
    observation.error = struct('category', categorize_error(exception.identifier));
end
end

function normalized = normalize_value(value, schema_version)
if schema_version == 1
    normalized = normalize_value_v1(value);
else
    state = new_normalization_state();
    [normalized, ~] = normalize_value_v2(value, state, 0);
end
end

function normalized = normalize_value_v1(value)
normalized = common_metadata(value);

if islogical(value)
    normalized.kind = 'logical';
    normalized.logical = reshape(value, 1, []);
elseif isinteger(value)
    normalized.kind = 'integer';
    normalized.integer = normalize_integers(value);
elseif isnumeric(value)
    normalized.kind = 'numeric';
    normalized.real = normalize_floats(real(value));
    normalized.imag = normalize_floats(imag(value));
elseif ischar(value)
    normalized.kind = 'char';
    normalized.code_units = reshape(double(value), 1, []);
elseif isstring(value)
    normalized.kind = 'string';
    [normalized.string, normalized.missing] = normalize_strings(value);
elseif iscell(value)
    normalized.kind = 'cell';
    normalized.items = cell(1, numel(value));
    for index = 1:numel(value)
        normalized.items{index} = normalize_value_v1(value{index});
    end
elseif isstruct(value)
    normalized.kind = 'struct';
    normalized.fields = fieldnames(value).';
    normalized.records = cell(1, numel(value));
    for record_index = 1:numel(value)
        record = struct();
        for field_index = 1:numel(normalized.fields)
            field_name = normalized.fields{field_index};
            record.(field_name) = normalize_value_v1( ...
                value(record_index).(field_name));
        end
        normalized.records{record_index} = record;
    end
elseif isobject(value)
    normalized.kind = 'object';
else
    error('OpenMatOracle:UnsupportedValue', ...
        'Probe returned a value kind outside the observation schema.');
end
end

function [normalized, state] = normalize_value_v2(value, state, depth)
kind = exact_value_kind(value);
normalized = common_metadata(value);
normalized.complex = isnumeric(value) && ~isreal(value);
validate_exact_metadata(normalized, kind, depth);
state = charge_budget(state, 'nodes', 1, 16384);
state = charge_budget(state, 'elements', normalized.numel, 65536);
state.depth = max(state.depth, depth);

if strcmp(kind, 'logical')
    normalized.kind = 'logical';
    normalized.logical = num2cell(reshape(value, 1, []));
elseif strcmp(kind, 'integer')
    normalized.kind = 'integer';
    normalized.integer = normalize_integer_components(value);
elseif strcmp(kind, 'numeric')
    normalized.kind = 'numeric';
    normalized.real = normalize_floats(real(value));
    normalized.imag = normalize_floats(imag(value));
elseif strcmp(kind, 'char')
    normalized.kind = 'char';
    normalized.code_units = num2cell(reshape(uint16(value), 1, []));
    state = charge_budget(state, 'code_units', ...
        numel(normalized.code_units), 65536);
elseif strcmp(kind, 'string')
    normalized.kind = 'string';
    [normalized.string_code_units, normalized.missing] = ...
        normalize_string_code_units(value);
    for index = 1:numel(normalized.string_code_units)
        element_code_units = numel(normalized.string_code_units{index});
        if element_code_units > 16384
            error('OpenMatOracle:PayloadLimit', ...
                'A string element exceeds the exact observation limit.');
        end
        state = charge_budget(state, 'code_units', ...
            element_code_units, 65536);
    end
elseif strcmp(kind, 'cell')
    normalized.kind = 'cell';
    normalized.items = cell(1, numel(value));
    for index = 1:numel(value)
        [normalized.items{index}, state] = normalize_value_v2( ...
            value{index}, state, depth + 1);
    end
elseif strcmp(kind, 'struct')
    normalized.kind = 'struct';
    normalized.fields = fieldnames(value).';
    for field_index = 1:numel(normalized.fields)
        field_name = normalized.fields{field_index};
        if isempty(regexp(field_name, ...
                '^[A-Za-z][A-Za-z0-9_]*$', 'once'))
            error('OpenMatOracle:UnsupportedPayload', ...
                'A struct field is outside the accepted identifier subset.');
        end
        state = charge_budget(state, 'code_units', ...
            numel(uint16(field_name)), 65536);
    end
    normalized.records = cell(1, numel(value));
    for record_index = 1:numel(value)
        record = struct();
        for field_index = 1:numel(normalized.fields)
            field_name = normalized.fields{field_index};
            [record.(field_name), state] = normalize_value_v2( ...
                value(record_index).(field_name), state, depth + 1);
        end
        normalized.records{record_index} = record;
    end
elseif strcmp(kind, 'table')
    normalized.kind = 'table';
    normalized.variableNames = value.Properties.VariableNames;
    if numel(normalized.variableNames) ~= width(value)
        error('OpenMatOracle:InvalidObservation', ...
            'A table variable schema is inconsistent with its width.');
    end
    normalized.variables = cell(1, numel(normalized.variableNames));
    for variable_index = 1:numel(normalized.variableNames)
        variable_name = normalized.variableNames{variable_index};
        if isempty(variable_name)
            error('OpenMatOracle:InvalidObservation', ...
                'A table variable name cannot be empty.');
        end
        state = charge_budget(state, 'code_units', ...
            numel(uint16(variable_name)), 65536);

        variable = value.(variable_name);
        if size(variable, 1) ~= height(value)
            error('OpenMatOracle:InvalidObservation', ...
                'A table variable has an inconsistent first dimension.');
        end
        [normalized.variables{variable_index}, state] = ...
            normalize_value_v2(variable, state, depth + 1);
    end
end
end

function state = new_normalization_state()
state = struct('nodes', 0, 'elements', 0, ...
    'code_units', 0, 'depth', 0);
end

function kind = exact_value_kind(value)
if islogical(value)
    kind = 'logical';
elseif isinteger(value)
    kind = 'integer';
elseif isnumeric(value) && (isa(value, 'double') || isa(value, 'single'))
    kind = 'numeric';
elseif ischar(value)
    kind = 'char';
elseif isstring(value)
    kind = 'string';
elseif iscell(value)
    kind = 'cell';
elseif isstruct(value)
    kind = 'struct';
elseif istable(value)
    kind = 'table';
else
    error('OpenMatOracle:UnsupportedPayload', ...
        'Probe returned an unsupported exact value kind.');
end
end

function validate_exact_metadata(metadata, kind, depth)
if depth > 32
    error('OpenMatOracle:PayloadLimit', ...
        'Exact observation depth exceeds the accepted limit.');
end

dimensions = double(metadata.size);
if numel(dimensions) < 2 || metadata.ndims ~= numel(dimensions)
    error('OpenMatOracle:InvalidObservation', ...
        'MATLAB returned inconsistent shape metadata.');
end

product = 1;
for index = 1:numel(dimensions)
    dimension = dimensions(index);
    if ~isfinite(dimension) || dimension < 0 || ...
            dimension ~= fix(dimension) || dimension > flintmax
        error('OpenMatOracle:PayloadLimit', ...
            'A shape dimension is outside the safe integer range.');
    end
    if dimension ~= 0 && product > flintmax / dimension
        error('OpenMatOracle:PayloadLimit', ...
            'A shape product is outside the safe integer range.');
    end
    product = product * dimension;
end
if product ~= metadata.numel
    error('OpenMatOracle:InvalidObservation', ...
        'MATLAB returned inconsistent element-count metadata.');
end
if metadata.numel > 4096
    error('OpenMatOracle:PayloadLimit', ...
        'An exact value node exceeds the accepted element limit.');
end

expected_class = switch_exact_class(kind, metadata.class);
if ~expected_class
    error('OpenMatOracle:InvalidObservation', ...
        'MATLAB returned inconsistent class metadata.');
end
end

function matches = switch_exact_class(kind, class_name)
switch kind
    case 'numeric'
        matches = strcmp(class_name, 'double') || strcmp(class_name, 'single');
    case 'integer'
        matches = any(strcmp(class_name, { ...
            'int8', 'uint8', 'int16', 'uint16', ...
            'int32', 'uint32', 'int64', 'uint64'}));
    otherwise
        matches = strcmp(class_name, kind);
end
end

function state = charge_budget(state, counter, amount, limit)
if amount < 0 || amount ~= fix(amount)
    error('OpenMatOracle:InvalidObservation', ...
        'A normalization counter increment is invalid.');
end
if state.(counter) > limit - amount
    error('OpenMatOracle:PayloadLimit', ...
        'The exact observation exceeds an aggregate budget.');
end
state.(counter) = state.(counter) + amount;
end

function metadata = common_metadata(value)
metadata = struct();
metadata.class = class(value);
metadata.size = size(value);
metadata.ndims = ndims(value);
metadata.numel = numel(value);
end

function values = normalize_floats(input)
flat = reshape(input, 1, []);
values = cell(1, numel(flat));
if isa(input, 'single')
    format = '%.9g';
else
    format = '%.17g';
end

for index = 1:numel(flat)
    item = flat(index);
    if isnan(item)
        values{index} = 'NaN';
    elseif isinf(item) && item > 0
        values{index} = '+Inf';
    elseif isinf(item)
        values{index} = '-Inf';
    else
        values{index} = sprintf(format, item);
    end
end
end

function values = normalize_integers(input)
flat = reshape(input, 1, []);
values = cell(1, numel(flat));
for index = 1:numel(flat)
    values{index} = char(string(flat(index)));
end
end

function values = normalize_integer_components(input)
flat = reshape(input, 1, []);
values = cell(1, numel(flat));
input_is_complex = ~isreal(input);
for index = 1:numel(flat)
    item = struct();
    item.real = integer_decimal(real(flat(index)));
    if input_is_complex
        item.imaginary = integer_decimal(imag(flat(index)));
    else
        item.imaginary = '0';
    end
    values{index} = item;
end
end

function text = integer_decimal(value)
% STRING formats integer inputs without a binary floating-point conversion.
text = char(string(value));
end

function [values, missing] = normalize_strings(input)
flat = reshape(input, 1, []);
missing = reshape(ismissing(flat), 1, []);
values = cell(1, numel(flat));
for index = 1:numel(flat)
    if missing(index)
        values{index} = '';
    else
        values{index} = char(flat(index));
    end
end
end

function [values, missing] = normalize_string_code_units(input)
flat = reshape(input, 1, []);
missing_flags = reshape(ismissing(flat), 1, []);
missing = num2cell(missing_flags);
values = cell(1, numel(flat));
for index = 1:numel(flat)
    if missing_flags(index)
        values{index} = cell(1, 0);
    else
        code_units = reshape(uint16(char(flat(index))), 1, []);
        values{index} = num2cell(code_units);
    end
end
end

function category = categorize_error(identifier)
identifier = lower(char(identifier));

if strcmp(identifier, 'openmatoracle:payloadlimit')
    category = 'payload-limit';
elseif strcmp(identifier, 'openmatoracle:unsupportedpayload') || ...
        strcmp(identifier, 'openmatoracle:unsupportedvalue')
    category = 'unsupported-payload';
elseif strcmp(identifier, 'openmatoracle:cyclicpayload')
    category = 'cyclic-payload';
elseif strcmp(identifier, 'openmatoracle:invalidobservation')
    category = 'invalid-observation';
elseif contains(identifier, 'prohibited') || contains(identifier, 'private') || ...
        contains(identifier, 'protected') || contains(identifier, 'access')
    category = 'access-violation';
elseif contains(identifier, 'dimagree') || contains(identifier, 'innerdim') || ...
        contains(identifier, 'dimmismatch') || ...
        contains(identifier, 'matrixdimensions') || contains(identifier, 'dimensions')
    category = 'dimension-mismatch';
elseif contains(identifier, 'badsubscript') || contains(identifier, 'index') || ...
        contains(identifier, 'subscript')
    category = 'index-out-of-bounds';
elseif contains(identifier, 'undefined') || contains(identifier, 'unrecognized')
    category = 'undefined-name';
elseif contains(identifier, 'maxlhs') || contains(identifier, 'minlhs') || ...
        contains(identifier, 'maxrhs') || contains(identifier, 'minrhs') || ...
        contains(identifier, 'notenough') || contains(identifier, 'toomany')
    category = 'arity';
elseif contains(identifier, 'assert')
    category = 'assertion-failed';
elseif contains(identifier, 'parse') || contains(identifier, 'syntax')
    category = 'syntax-error';
elseif contains(identifier, 'type') || contains(identifier, 'conversion') || ...
        contains(identifier, 'class')
    category = 'type-error';
else
    category = 'other';
end
end

function [observation, json_text] = encode_bounded_observation(observation)
json_text = jsonencode(observation, 'PrettyPrint', true);
encoded = unicode2native([json_text, newline], 'UTF-8');
if numel(encoded) <= 1048576
    return;
end

observation = normalization_error_observation(observation, 'payload-limit');
json_text = jsonencode(observation, 'PrettyPrint', true);
encoded = unicode2native([json_text, newline], 'UTF-8');
if numel(encoded) > 1048576
    error('OpenMatOracle:EncodedErrorLimit', ...
        'The mandatory normalization error exceeds the encoded limit.');
end
end

function result = normalization_error_observation(observation, category)
result = struct();
result.schema_version = observation.schema_version;
result.case_id = observation.case_id;
result.oracle = observation.oracle;
result.outcome = 'error';
result.error = struct('category', category);
end

function write_json(output_path, json_text)
file = fopen(output_path, 'w', 'n', 'UTF-8');
if file < 0
    error('OpenMatOracle:OutputOpenFailed', ...
        'Could not open the normalized output file.');
end
file_cleanup = onCleanup(@() fclose(file)); %#ok<NASGU>
fprintf(file, '%s\n', json_text);
end
