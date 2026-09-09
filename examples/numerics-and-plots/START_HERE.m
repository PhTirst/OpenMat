%% 从这里开始 / Start here
% 点击编辑器上方的 Run，生成函数曲线图。
% 其他示例见同目录 README.txt：FFT、曲线拟合，以及彩色二维和三维图形。
% 每个 .m 文件都可单独运行，无需先运行本文件。
% Click Run to draw the curves. Try the other scripts in this folder next.

x = linspace(0, 2*pi, 200);

y1 = sin(x);
y2 = cos(x);
y3 = exp(-0.2*x) .* sin(2*x);

figure('Name', 'Start Here - Function Curves');

plot(x, y1, 'b-', ...
    'LineWidth', 2);
hold on;

plot(x, y2, 'r--', ...
    'LineWidth', 2);

plot(x, y3, 'ko-', ...
    'LineWidth', 1.5, ...
    'MarkerSize', 4, ...
    'MarkerIndices', 1:10:length(x));

hold off;

xlabel('$x$', 'Interpreter', 'latex');
ylabel('$f(x)$', 'Interpreter', 'latex');

title('2D Function Plot', ...
    'Interpreter', 'latex');

legend( ...
    '$\sin(x)$', ...
    '$\cos(x)$', ...
    '$e^{-0.2x}\sin(2x)$', ...
    'Interpreter', 'latex', ...
    'Location', 'best');

xlim([0, 2*pi]);
ylim([-1.2, 1.2]);

xticks(0:pi/2:2*pi);
xticklabels({ ...
    '$0$', ...
    '$\pi/2$', ...
    '$\pi$', ...
    '$3\pi/2$', ...
    '$2\pi$'});

set(gca, 'TickLabelInterpreter', 'latex');

grid on;
box on;

set(gca, ...
    'FontSize', 12, ...
    'LineWidth', 1);
