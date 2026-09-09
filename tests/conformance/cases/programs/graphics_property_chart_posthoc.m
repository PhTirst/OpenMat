figure_handle = figure('Visible', 'off');
checks = zeros(1, 47);

chart_handle = stairs(1:3, [2 5 1]);
set(chart_handle, 'Color', [0.1 0.2 0.3], 'LineWidth', 2.5, 'LineStyle', '--', ...
    'Marker', 'x', 'MarkerSize', 8, 'MarkerFaceColor', [0.4 0.5 0.6], ...
    'MarkerEdgeColor', [0.7 0.8 0.9]);
checks(1) = all(get(chart_handle, 'Color') == [0.1 0.2 0.3]);
checks(2) = get(chart_handle, 'linew') == 2.5;
checks(3) = strcmp(get(chart_handle, 'lines'), '--');
checks(4) = strcmp(get(chart_handle, 'Marker'), 'x');
checks(5) = get(chart_handle, 'MarkerSize') == 8;
checks(6) = all(get(chart_handle, 'MarkerFaceColor') == [0.4 0.5 0.6]);
checks(7) = all(get(chart_handle, 'MarkerEdgeColor') == [0.7 0.8 0.9]);
set(chart_handle, 'MarkerFaceColor', 'auto', 'MarkerEdgeColor', 'none');
checks(43) = strcmp(get(chart_handle, 'MarkerFaceColor'), 'auto');
checks(44) = strcmp(get(chart_handle, 'MarkerEdgeColor'), 'none');
set(chart_handle, 'Color', [0.2 0.3 0.4]);
checks(45) = strcmp(get(chart_handle, 'MarkerFaceColor'), 'auto');

chart_handle = stem(1:3, [2 5 1]);
set(chart_handle, 'Color', [0.1 0.2 0.3], 'LineWidth', 2.5, 'LineStyle', ':', ...
    'Marker', 'square', 'MarkerSize', 8, 'MarkerFaceColor', [0.4 0.5 0.6], ...
    'MarkerEdgeColor', [0.7 0.8 0.9], 'BaseValue', -2);
checks(8) = all(get(chart_handle, 'Color') == [0.1 0.2 0.3]);
checks(9) = get(chart_handle, 'LineWidth') == 2.5;
checks(10) = strcmp(get(chart_handle, 'LineStyle'), ':');
checks(11) = strcmp(get(chart_handle, 'Marker'), 'square');
checks(12) = get(chart_handle, 'MarkerSize') == 8;
checks(13) = all(get(chart_handle, 'MarkerFaceColor') == [0.4 0.5 0.6]);
checks(14) = all(get(chart_handle, 'MarkerEdgeColor') == [0.7 0.8 0.9]);
checks(15) = get(chart_handle, 'BaseValue') == -2;

chart_handle = errorbar(1:3, [2 5 1], [0.2 0.3 0.1]);
set(chart_handle, 'Color', [0.1 0.2 0.3], 'LineWidth', 2.5, 'LineStyle', '-.', ...
    'Marker', 'o', 'MarkerSize', 8, 'CapSize', 12);
checks(16) = all(get(chart_handle, 'Color') == [0.1 0.2 0.3]);
checks(17) = get(chart_handle, 'LineWidth') == 2.5;
checks(18) = strcmp(get(chart_handle, 'LineStyle'), '-.');
checks(19) = strcmp(get(chart_handle, 'Marker'), 'o');
checks(20) = get(chart_handle, 'MarkerSize') == 8;
checks(21) = get(chart_handle, 'CapSize') == 12;

chart_handle = area(1:3, [2 5 1]);
set(chart_handle, 'FaceColor', [0.1 0.2 0.3], 'EdgeColor', [0.7 0.8 0.9], ...
    'LineWidth', 2.5, 'LineStyle', '--', 'FaceAlpha', 0.4, 'EdgeAlpha', 0.6, ...
    'BaseValue', -2);
checks(22) = all(get(chart_handle, 'FaceColor') == [0.1 0.2 0.3]);
checks(23) = all(get(chart_handle, 'EdgeColor') == [0.7 0.8 0.9]);
checks(24) = get(chart_handle, 'LineWidth') == 2.5;
checks(25) = strcmp(get(chart_handle, 'LineStyle'), '--');
checks(26) = abs(get(chart_handle, 'FaceAlpha') - 0.4) < 1e-6;
checks(27) = abs(get(chart_handle, 'EdgeAlpha') - 0.6) < 1e-6;
checks(28) = get(chart_handle, 'BaseValue') == -2;

chart_handle = bar(1:3, [2 5 1]);
set(chart_handle, 'FaceColor', [0.1 0.2 0.3], 'EdgeColor', [0.7 0.8 0.9], ...
    'LineWidth', 2.5, 'LineStyle', ':', 'FaceAlpha', 0.4, 'EdgeAlpha', 0.6, ...
    'BaseValue', -2, 'BarWidth', 0.5);
checks(29) = all(get(chart_handle, 'FaceColor') == [0.1 0.2 0.3]);
checks(30) = all(get(chart_handle, 'EdgeColor') == [0.7 0.8 0.9]);
checks(31) = get(chart_handle, 'LineWidth') == 2.5;
checks(32) = strcmp(get(chart_handle, 'LineStyle'), ':');
checks(33) = abs(get(chart_handle, 'FaceAlpha') - 0.4) < 1e-6;
checks(34) = abs(get(chart_handle, 'EdgeAlpha') - 0.6) < 1e-6;
checks(35) = get(chart_handle, 'BaseValue') == -2;
checks(36) = get(chart_handle, 'BarWidth') == 0.5;

chart_handle = histogram([1 1 2 3 3 3]);
set(chart_handle, 'FaceColor', [0.1 0.2 0.3], 'EdgeColor', [0.7 0.8 0.9], ...
    'LineWidth', 2.5, 'LineStyle', '-.', 'FaceAlpha', 0.4, 'EdgeAlpha', 0.6);
checks(37) = all(get(chart_handle, 'FaceColor') == [0.1 0.2 0.3]);
checks(38) = all(get(chart_handle, 'EdgeColor') == [0.7 0.8 0.9]);
checks(39) = get(chart_handle, 'LineWidth') == 2.5;
checks(40) = strcmp(get(chart_handle, 'LineStyle'), '-.');
checks(41) = abs(get(chart_handle, 'FaceAlpha') - 0.4) < 1e-6;
checks(42) = abs(get(chart_handle, 'EdgeAlpha') - 0.6) < 1e-6;
set(chart_handle, 'FaceColor', 'auto', 'EdgeColor', 'auto');
checks(46) = strcmp(get(chart_handle, 'FaceColor'), 'auto');
checks(47) = strcmp(get(chart_handle, 'EdgeColor'), 'auto');

openmat_result = sum(checks);
close(figure_handle);
