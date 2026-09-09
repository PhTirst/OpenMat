% 打开本文件并点击 Run。可修改下方采样量、参数或颜色。
% Open this script and click Run. No extra data or toolbox is required.

N = 3500;

n = 1:N;
golden_angle = pi * (3 - sqrt(5));

theta = n * golden_angle;
r = sqrt(n);

x = r .* cos(theta);
y = r .* sin(theta);

c = mod(theta, 2*pi);

figure('Color', [0.03 0.03 0.04]);
scatter(x, y, 22, c, 'filled');

axis equal;
axis off;

colormap(turbo);

title('Golden Phyllotaxis', ...
    'Color', 'w', ...
    'FontSize', 18);
