assert(rust_add(2, 3) == 5);
a = [1, 2; 3, 4];
b = rust_scale(a, 2);
assert(isequal(a, [1, 2; 3, 4]));
assert(isequal(b, [2, 4; 6, 8]));
assert(isequal(full(rust_sparse()), [2, 0; 0, 3]));
assert(rust_invoke(@(x) x + 3, 4) == 7);
assert(rust_selftest() == 1);
[changed, retained] = rust_taken(a);
assert(isequal(changed, [99, 99; 99, 99]));
assert(isequal(retained, a));
assert(rust_duplicate_output() == 1);

c = RustCounter(3);
c.increment(2);
assert(c.Value == 5);
c.Value = 8;
assert(c.Value == 8);
clear c;

failed = false;
try
    rust_failure();
catch err
    failed = strcmp(err.identifier, 'RustTest:Failure');
end
assert(failed);
failed = false;
try
    rust_panic();
catch err
    failed = strcmp(err.identifier, 'OpenMat:OEX:RustPanic');
end
assert(failed);
assert(rust_selftest() == 1);

assert(rust_callback(@(x) x * 3, 4) == 12);
[x, y] = rust_callback(@two_outputs, 5);
assert(x == 5 && y == 6);
rust_callback(@no_outputs, 5);
failed = false;
try
    rust_swallow(@language_failure);
catch err
    failed = strcmp(err.identifier, 'RustTest:Language');
end
assert(failed);

before = rust_destroyed();
failed = false;
try
    rust_failed_factory();
catch
    failed = true;
end
assert(failed);
assert(rust_destroyed() == before + 2);
c = rust_factory();
assert(c.Value == 12);
assert(rust_read_object(c) == 12);
c.increment(5);
assert(c.Value == 17);
failed = false;
try
    c.reenter(@() c.increment(1));
catch
    failed = true;
end
assert(failed);
assert(c.Value == 17);
clear c;
assert(rust_destroyed() == before + 3);
c = RustTestCounter(23);
assert(c.Value == 23);
c.Value = 24;
assert(c.Value == 24);
clear c;
assert(rust_destroyed() == before + 4);

function [a, b] = two_outputs(x)
a = x;
b = x + 1;
end

function no_outputs(x)
assert(x == 5);
end

function language_failure()
error('RustTest:Language', 'original language failure');
end
