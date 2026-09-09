%% 最小二乘拟合 / Least-squares curve fitting
% 打开本文件并点击 Run。矩阵左除同时求出二次项、一次项和常数项。
% 用固定的扰动生成数据，每次运行都能复现同一结果。

x = linspace(-3, 3, 41)';
y = 0.65*x.^2 - 0.4*x + 1.2 + 0.32*sin(5*x);
design = [x.^2, x, ones(size(x))];
coefficients = design \ y;
fitted = design * coefficients;
residual = y - fitted;
rmse = sqrt(mean(residual.^2));

dense_x = linspace(-3, 3, 400)';
dense_y = [dense_x.^2, dense_x, ones(size(dense_x))] * coefficients;

figure('Name', 'Least Squares Fit', 'Color', [0.98 0.98 1]);
subplot(2, 1, 1);
plot(x, y, 'o', 'Color', [0.85 0.45 0.27], 'MarkerSize', 5);
set(gca, 'Position', [0.14 0.61 0.8 0.23]);
hold on;
plot(dense_x, dense_y, 'Color', [0.13 0.55 0.65], 'LineWidth', 2.2);
hold off;
legend('Measurements', 'Quadratic fit', 'Location', 'best');
xlabel('x');
ylabel('y');
title('Quadratic least squares');
grid on;

subplot(2, 1, 2);
plot(x, residual, 'o-', 'Color', [0.48 0.35 0.78], 'LineWidth', 1.4);
set(gca, 'Position', [0.14 0.16 0.8 0.23]);
hold on;
plot([-3 3], [0 0], '--', 'Color', [0.45 0.5 0.55]);
hold off;
xlabel('x');
ylabel('Residual');
title('Fitting residuals');
grid on;

disp('Quadratic, linear and constant coefficients:');
disp(coefficients);
disp('Root mean square error:');
disp(rmse);
