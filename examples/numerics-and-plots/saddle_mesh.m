%% 鞍形线框 / Saddle wire mesh
% mesh 绘制规则网格线；FaceColor='none' 让网格之间完全透明。
% 改变采样点数，可观察线框密度与鞍形起伏。

[X, Y] = meshgrid(linspace(-5, 5, 49));
Z = 0.22*(X.^2 - Y.^2).*exp(-0.035*(X.^2 + Y.^2));

figure('Name', 'Saddle Wire Mesh', 'Color', [0.02 0.03 0.05]);
mesh(X, Y, Z, 'FaceColor', 'none', 'EdgeColor', 'flat', 'LineWidth', 0.7);
axis tight;
axis off;
colormap(turbo);
view(42, 30);
title('Saddle | mesh', 'Color', 'w', 'FontSize', 18);
