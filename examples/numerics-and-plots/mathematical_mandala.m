% 打开本文件并点击 Run。可修改下方采样量、参数或颜色。
% Open this script and click Run. No extra data or toolbox is required.

figure('Color', [0.02 0.02 0.03]);
hold on;

t = linspace(0, 2*pi, 1200);

N = 28;

for k = 1:N
    phase = 2*pi*k/N;

    r = 2 ...
        + 0.45*sin(7*t + phase) ...
        + 0.22*sin(13*t - 2*phase);

    x = r .* cos(t);
    y = r .* sin(t);

    plot(x, y, ...
        'LineWidth', 0.8);
end

axis equal;
axis off;

title('Mathematical Mandala', ...
    'Color', 'w', ...
    'FontSize', 18);
