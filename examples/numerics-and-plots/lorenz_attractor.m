% 打开本文件并点击 Run。可修改下方采样量、参数或颜色。
% Open this script and click Run. No extra data or toolbox is required.

sigma = 10;
rho = 28;
beta = 8/3;

dt = 0.003;
N = 12000;

x = zeros(1,N);
y = zeros(1,N);
z = zeros(1,N);

x(1) = 0.1;
y(1) = 0;
z(1) = 0;

for k = 1:N-1
    dx = sigma*(y(k)-x(k));
    dy = x(k)*(rho-z(k))-y(k);
    dz = x(k)*y(k)-beta*z(k);

    x(k+1) = x(k) + dt*dx;
    y(k+1) = y(k) + dt*dy;
    z(k+1) = z(k) + dt*dz;
end

figure('Color', [0.02 0.02 0.025]);

plot3(x, y, z, ...
    'LineWidth', 0.9);

axis off;
axis tight;

view(30, 18);

title('Lorenz Attractor', ...
    'Color', 'w', ...
    'FontSize', 18);
