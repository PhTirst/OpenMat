%% RCS 极坐标方向图 / Normalized monostatic RCS
% 三个各向同性点散射体，在远场作标量相干叠加，忽略相互耦合。
% 往返路径产生 2*k*r 的相位。半径为 RCS / (sum(amplitude)^2)，
% 是无量纲线性值，不是 dBsm。改变散射体位置或波长，观察波瓣变化。
% 模型背景（本例参数和实现由 OpenMat 独立编写）：
% https://www.mathworks.com/help/radar/ug/modeling-target-radar-cross-section.html

theta = linspace(0, 2*pi, 1081);
positions = [-0.40 0.10; 0.20 0.35; 0.45 -0.30]; % metres
amplitudes = [1.0 0.8 0.55];
wavelengths = [0.90 0.55];                     % metres
normalizedRcs = zeros(2, numel(theta));
coherentPeak = sum(amplitudes)^2;

for curve = 1:2
    waveNumber = 2*pi/wavelengths(curve);
    echo = zeros(size(theta));
    for scatterer = 1:size(positions, 1)
        distance = positions(scatterer, 1)*cos(theta) ...
            + positions(scatterer, 2)*sin(theta);
        phase = 2*waveNumber*distance;
        echo = echo + amplitudes(scatterer)*(cos(phase) + 1i*sin(phase));
    end
    normalizedRcs(curve, :) = abs(echo).^2 / coherentPeak;
end

figure('Name', 'RCS Polar Pattern', 'Color', [0.975 0.985 1]);
polarplot(theta, normalizedRcs(1, :), 'Color', [0.08 0.57 0.64], 'LineWidth', 2.2);
hold on;
polarplot(theta, normalizedRcs(2, :), '--', 'Color', [0.66 0.35 0.75], 'LineWidth', 1.8);
rlim([0 1]);
rticks([0.25 0.5 0.75 1]);
rticklabels({'0.25', '0.50', '0.75', '1.00'});
thetaticks(0:45:360);
set(gca, 'Position', [0.10 0.18 0.80 0.70], 'ThetaZeroLocation', 'top', ...
    'ThetaDir', 'clockwise', 'FontSize', 10);
title('Normalized RCS', 'FontSize', 17);
legend({'Wavelength 0.90 m', 'Wavelength 0.55 m'}, ...
    'Location', 'southoutside', 'Orientation', 'horizontal', 'FontSize', 9);
disp('RCS: linear normalized power; each curve includes the round-trip phase.');
