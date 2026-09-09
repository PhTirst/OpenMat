function component = create(type, varargin)
component = openmat.ui.Control(type);
if strcmp(type, 'NumericField') || strcmp(type, 'Slider') || strcmp(type, 'ProgressBar')
    component.Value = 0;
elseif strcmp(type, 'CheckBox')
    component.Value = false;
end
first = 1;
if nargin > 1 && isa(varargin{1}, 'matlab.ui.componentcontainer.ComponentContainer')
    parent = varargin{1};
    parent.add(component);
    first = 2;
end
for index = first:2:numel(varargin)
    name = varargin{index};
    component.(name) = varargin{index + 1};
end
end
