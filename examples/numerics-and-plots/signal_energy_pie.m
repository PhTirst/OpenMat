%% 信号能量占比饼图 / Energy shares of discrete signal components
% 四个整周期正弦分量互相正交，能量可相加，扇区角度正比于分量能量。
% 这里使用 fill 构造扇区，不依赖尚未提供的 pie 函数。
% 改变 amplitudes，重新运行即可比较能量占比；能量与振幅平方成正比。

sampleRate = 512;
sampleCount = 512;
time = (0:sampleCount-1)/sampleRate;
frequencies = [16 48 96 160];
amplitudes = [1.0 0.75 0.50 0.35];
components = zeros(4, sampleCount);
for component = 1:4
    components(component, :) = amplitudes(component)*sin(2*pi*frequencies(component)*time);
end
componentEnergy = sum(components.^2, 2);
energyShare = componentEnergy/sum(componentEnergy);
combinedSignal = sum(components, 1);
palette = [0.09 0.64 0.66; 0.51 0.41 0.80; 0.96 0.66 0.28; 0.88 0.39 0.49];
labels = cell(1, 4);

figure('Name', 'Signal Energy Pie', 'Color', [0.98 0.985 1]);
hold on;
startAngle = pi/2;
for component = 1:4
    endAngle = startAngle + 2*pi*energyShare(component);
    arc = linspace(startAngle, endAngle, 100);
    middle = (startAngle+endAngle)/2;
    % 将最大分量稍微移出，突出主导频率。
    offset = 0.07*(component == 1);
    centerX = offset*cos(middle); centerY = offset*sin(middle);
    fill(centerX+[0 cos(arc) 0], centerY+[0 sin(arc) 0], palette(component, :), ...
        'EdgeColor', [0.98 0.985 1], 'LineWidth', 2);
    labels{component} = sprintf('%d Hz: %.1f%%', frequencies(component), 100*energyShare(component));
    startAngle = endAngle;
end
axis equal;
axis off;
xlim([-1.2 1.2]); ylim([-1.2 1.2]);
set(gca, 'Position', [0.12 0.22 0.76 0.66]);
title('Energy Share', 'FontSize', 17);
legend(labels, 'Location', 'southoutside', 'Orientation', 'horizontal', 'FontSize', 9);
disp('Energy shares (%):');
disp(100*energyShare');
