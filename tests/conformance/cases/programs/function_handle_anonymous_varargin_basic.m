collect = @(varargin) varargin;
empty_inputs = collect();
many_inputs = collect(4, 5, 6);
pick = @(varargin) varargin{2};
count = @(varargin) nargin;
forward = @(varargin) openmat_variadic_inputs(varargin{:});
forwarded = forward(4, 5, 6);

openmat_result = [size(empty_inputs, 1), size(empty_inputs, 2), ...
    size(many_inputs, 1), size(many_inputs, 2), ...
    many_inputs{1}, many_inputs{3}, pick(7, 9), ...
    count(), count(1, 2, 3), forwarded];
