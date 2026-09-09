figure;
figure_handle = gcf();
on = 91;
off = 92;

initial_hold = ishold();
hold on;
command_hold_on = ishold();
hold off;
command_hold_off = ishold();

hold('on');
functional_hold_on = ishold();
hold('off');
functional_hold_off = ishold();

grid on % trailing command comment
command_grid_on = strcmp(get(gca(), 'XGrid'), 'on');
grid off;
command_grid_off = strcmp(get(gca(), 'XGrid'), 'off');

strcmp alpha alpha;
plain_text_equal = ans;
strcmp alpha ... command continuation comment
    alpha;
continued_text_equal = ans;
strcmp 'alpha beta' 'alpha beta';
quoted_text_equal = ans;
strcmp "alpha" '"alpha"';
double_quote_is_text = ans;

close(figure_handle);
openmat_result = [on, off, initial_hold, command_hold_on, ...
    command_hold_off, functional_hold_on, functional_hold_off, ...
    command_grid_on, command_grid_off, plain_text_equal, ...
    continued_text_equal, quoted_text_equal, double_quote_is_text];
