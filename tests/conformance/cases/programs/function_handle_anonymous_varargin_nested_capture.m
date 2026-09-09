offset = 10;
captured = @(varargin) offset + varargin{1};
offset = 99;

outer = @(varargin) @(value) value + varargin{1};
inner = outer(3);
outer_reverse = @(value) @(varargin) value + varargin{1};
inner_reverse = outer_reverse(5);
shadow_factory = @(varargin) @(varargin) varargin;
shadow = shadow_factory(1, 2);
shadowed_values = shadow(8, 9);

openmat_result = [captured(2), inner(4), inner_reverse(6), ...
    size(shadowed_values, 1), size(shadowed_values, 2), ...
    shadowed_values{1}, shadowed_values{2}];
