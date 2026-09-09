% MATLAB Code Analyzer check for this standalone toolbox (MATLAB only).
% Emit only our filenames/line numbers and diagnostic IDs, not tool prose.
files = [dir('+fem/**/*.m'); dir('+femtests/*.m'); dir('+femexamples/*.m'); ...
    dir('run_*.m'); dir('demo_*.m')];
issues = 0;
for k = 1:numel(files)
    diagnostics = checkcode(fullfile(files(k).folder, files(k).name), '-id');
    for j = 1:numel(diagnostics)
        issues = issues + 1;
        disp([files(k).name, ':', num2str(diagnostics(j).line), ' ', diagnostics(j).id]);
    end
end
assert(issues == 0);
disp('FEM MATLAB code analysis passed.');
