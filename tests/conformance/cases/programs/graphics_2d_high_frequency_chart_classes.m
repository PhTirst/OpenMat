figure_handle = figure();
openmat_result = zeros(1, 6);

chart_handle = stairs(1:3, [2 5 1]);
openmat_result(1) = strcmp(class(chart_handle), 'matlab.graphics.chart.primitive.Stair');
chart_handle = stem(1:3, [2 5 1]);
openmat_result(2) = strcmp(class(chart_handle), 'matlab.graphics.chart.primitive.Stem');
chart_handle = errorbar(1:3, [2 5 1], [0.2 0.3 0.1]);
openmat_result(3) = strcmp(class(chart_handle), 'matlab.graphics.chart.primitive.ErrorBar');
chart_handle = area(1:3, [2 5 1]);
openmat_result(4) = strcmp(class(chart_handle), 'matlab.graphics.chart.primitive.Area');
chart_handle = bar(1:3, [2 5 1]);
openmat_result(5) = strcmp(class(chart_handle), 'matlab.graphics.chart.primitive.Bar');
chart_handle = histogram([1 1 2 3 3 3]);
openmat_result(6) = strcmp(class(chart_handle), 'matlab.graphics.chart.primitive.Histogram');

close(figure_handle);
