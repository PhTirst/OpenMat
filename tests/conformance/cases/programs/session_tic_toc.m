ok = true;

try
    toc;
    ok = false;
catch ME
    ok = ok && strcmp(ME.identifier, 'MATLAB:toc:callTicFirstNoInputs');
end

timer = tic;
elapsed = toc(timer);
ok = ok && isa(timer, 'uint64') && isscalar(timer);
ok = ok && isa(elapsed, 'double') && isscalar(elapsed) && elapsed >= 0;

try
    toc;
    ok = false;
catch ME
    ok = ok && strcmp(ME.identifier, 'MATLAB:toc:callTicFirstNoInputs');
end

tic;
default_elapsed = toc;
ok = ok && isa(default_elapsed, 'double') && default_elapsed >= 0;

try
    toc(double(timer));
    ok = false;
catch ME
    ok = ok && strcmp(ME.identifier, 'MATLAB:toc:wrongTocArgument');
end

try
    tic(1);
    ok = false;
catch ME
    ok = ok && strcmp(ME.identifier, 'MATLAB:maxrhs');
end

try
    [left, right] = toc(timer);
    ok = false;
catch ME
    ok = ok && strcmp(ME.identifier, 'MATLAB:maxlhs');
end

openmat_result = ok;
