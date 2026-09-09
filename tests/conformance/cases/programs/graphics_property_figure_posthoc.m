figure_handle = figure('Visible', 'off');

set(figure_handle, 'NaM', 'OpenMat property');
set(figure_handle, 'NumberT', 'off');
set(figure_handle, 'Pos', [101 202 640 480]);
set(figure_handle, 'Color', [0.1 0.2 0.3]);

checks = zeros(1, 4);
checks(1) = strcmp(get(figure_handle, 'NAME'), 'OpenMat property');
checks(2) = strcmp(get(figure_handle, 'numbertitle'), 'off');
checks(3) = all(get(figure_handle, 'POSITION') == [101 202 640 480]);
checks(4) = all(abs(get(figure_handle, 'color') - [0.1 0.2 0.3]) < 1e-6);
openmat_result = sum(checks);

close(figure_handle);
