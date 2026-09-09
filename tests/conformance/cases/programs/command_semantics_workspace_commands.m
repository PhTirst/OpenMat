clc_marker = 41;
clc;
clc_preserves_workspace = clc_marker == 41;

format long g;
long_options = format;
long_numeric = strcmp(char(long_options.NumericFormat), 'longG');
long_spacing = strcmp(char(long_options.LineSpacing), 'loose');

format compact;
compact_options = format;
compact_preserves_numeric = strcmp(char(compact_options.NumericFormat), 'longG');
compact_spacing = strcmp(char(compact_options.LineSpacing), 'compact');

format short;
short_options = format;
short_numeric = strcmp(char(short_options.NumericFormat), 'short');
short_preserves_spacing = strcmp(char(short_options.LineSpacing), 'compact');

format loose;
loose_options = format;
loose_spacing = strcmp(char(loose_options.LineSpacing), 'loose');

alpha = 11;
beta = single([2 3]);
gamma = true;
names = who('alpha', 'beta', 'gamma');
who_values = numel(names) == 3 && ...
    any(strcmp(names, 'alpha')) && any(strcmp(names, 'beta')) && ...
    any(strcmp(names, 'gamma'));

details = whos('beta');
whos_value = numel(details) == 1 && strcmp(details.name, 'beta') && ...
    strcmp(details.class, 'single') && isequal(details.size, [1 2]) && ...
    ~details.global && ~details.sparse && ~details.complex;

who alpha beta;
whos alpha beta;
display_forms_preserve_workspace = alpha == 11 && ...
    isequal(beta, single([2 3]));

clearvars_value = openmat_command_semantics_clearvars();

openmat_result = [ ...
    clc_preserves_workspace, long_numeric, long_spacing, ...
    compact_preserves_numeric, compact_spacing, short_numeric, ...
    short_preserves_spacing, loose_spacing, who_values, whos_value, ...
    display_forms_preserve_workspace, clearvars_value ...
];

function result = openmat_command_semantics_clearvars()
keep_value = 17;
drop_value = 29;
clearvars -except keep_value;
result = exist('keep_value', 'var') == 1 && ...
    exist('drop_value', 'var') == 0 && keep_value == 17;
end
