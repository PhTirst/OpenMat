figure_handle = figure();
initial_state = ishold();
hold('on');
enabled_state = ishold();
hold('off');
disabled_state = ishold();
openmat_result = [initial_state, enabled_state, disabled_state];
close(figure_handle);
