ok = true;

try
    error('Probe:Expected', 'value %d', 7);
    ok = false;
catch ME
    ok = ok && strcmp(class(ME), 'MException');
    ok = ok && strcmp(ME.identifier, 'Probe:Expected');
    ok = ok && strcmp(ME.message, 'value 7');
end

try
    assert(false, 'Probe:Assert', 'item %d', 3);
    ok = false;
catch ME
    ok = ok && strcmp(ME.identifier, 'Probe:Assert');
    ok = ok && strcmp(ME.message, 'item 3');
end

try
    assert([true true]);
    ok = false;
catch ME
    ok = ok && strcmp(ME.identifier, 'MATLAB:assertion:LogicalScalar');
end

error('');
assert(true);
assert(single(1));
assert(uint8(1));

openmat_result = ok;
