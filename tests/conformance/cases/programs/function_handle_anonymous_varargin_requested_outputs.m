handle = @(varargin) openmat_multi_output(varargin{:});
single = handle(2);
[first, second, third] = handle(3);

target = @openmat_multi_output;
factory = @(target) @(varargin) target(varargin{:});
nested = factory(target);
[nested_first, nested_second, nested_third] = nested(4);

openmat_result = [single, first, second, third, ...
    nested_first, nested_second, nested_third];
