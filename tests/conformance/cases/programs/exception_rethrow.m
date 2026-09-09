ok = true;

try
    try
        error('Probe:RethrowNested', 'nested value %d', 4);
    catch inner
        rethrow(inner);
    end
    ok = false;
catch outer
    ok = ok && strcmp(class(outer), 'MException');
    ok = ok && strcmp(outer.identifier, 'Probe:RethrowNested');
    ok = ok && strcmp(outer.message, 'nested value 4');
end

try
    error('Probe:RethrowSaved', 'saved message');
catch captured
    saved = captured;
end

try
    rethrow(saved);
    ok = false;
catch repeated
    ok = ok && strcmp(repeated.identifier, 'Probe:RethrowSaved');
    ok = ok && strcmp(repeated.message, 'saved message');
end

try
    rethrow(1);
    ok = false;
catch invalid
    ok = ok && strcmp(invalid.identifier, 'MATLAB:rethrow:invalidInputType');
end

try
    output = rethrow(saved);
    ok = false;
catch invalid_output
    ok = ok && strcmp(invalid_output.identifier, 'MATLAB:UndefinedFunction');
end

openmat_result = ok;
