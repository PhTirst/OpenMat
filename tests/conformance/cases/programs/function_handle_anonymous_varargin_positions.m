first_named = @(varargin, tail) [varargin, tail, nargin];
middle_named = @(head, varargin, tail) [head, varargin, tail, nargin];
ignored_tail = @(varargin, ~) [varargin, nargin];
final_named = @(head, varargin) ...
    [head, nargin, numel(varargin), varargin{1}, varargin{2}];

openmat_result = [first_named(4, 5), middle_named(6, 7, 8), ...
    ignored_tail(9, 10), final_named(10, 11, 12)];
