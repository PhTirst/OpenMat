function result = openmat_variadic_inputs(head, varargin)
result = [nargin, numel(varargin), head];
if nargin > 1
    result = [result, varargin{1}];
end
if nargin > 2
    result = [result, varargin{2}];
end
end
