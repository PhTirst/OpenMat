function Probe_Mat73Stage2(outputDirectory)
%PROBE_MAT73STAGE2 Author locally-owned MATLAB R2022b MAT v7.3 fixtures.
% This probe records only values and assertions designed for OpenMat.

arguments
    outputDirectory (1, 1) string
end

if ~isfolder(outputDirectory)
    mkdir(outputDirectory);
end

sparseReal = sparse([1 3 2], [1 2 4], [1.5 -2 9], 3, 5, 8);
sparseComplex = sparse([2 4], [1 3], [3+4i -5+2i], 4, 3, 7);
sparseLogical = sparse(logical([1 0 1; 0 1 0]));
sparseEmptyRows = sparse(0, 4);
sparseEmptyColumns = sparse(3, 0);
sparseAllocatedEmpty = spalloc(5, 6, 11);
sparseLogicalEmpty = sparse(false(2, 0));
structSparse = struct('left', sparseReal, 'right', sparseLogical);

save(fullfile(outputDirectory, 'matlab-stage2-sparse-v73.mat'), ...
    'sparseReal', 'sparseComplex', 'sparseLogical', ...
    'sparseEmptyRows', 'sparseEmptyColumns', 'sparseAllocatedEmpty', ...
    'sparseLogicalEmpty', ...
    'structSparse', '-v7.3');

stringScalar = "A" + string(char([hex2dec('D83D') hex2dec('DE03')])) + "B";
stringArray = ["alpha" missing; "" "omega"];
stringEmpty = strings(0, 3);

cellNested = {sparseReal, stringScalar; stringArray, {uint16(7)}};
structNested = struct('name', {"first", "second"}, ...
    'payload', {sparseComplex, stringArray});

save(fullfile(outputDirectory, 'matlab-stage2-v73.mat'), ...
    'sparseReal', 'sparseComplex', 'sparseLogical', ...
    'sparseEmptyRows', 'sparseEmptyColumns', 'sparseAllocatedEmpty', ...
    'stringScalar', 'stringArray', 'stringEmpty', ...
    'cellNested', 'structNested', '-v7.3');

unsupportedTable = table((1:2)', ["a"; "b"], ...
    'VariableNames', {'Number', 'Text'});
unsupportedDatetime = datetime(2022, 1, 2);
unsupportedCategorical = categorical(["red" "blue"]);
unsupportedClassdef = OpenMatProbeClass(uint32(17));
save(fullfile(outputDirectory, 'matlab-stage2-unsupported-v73.mat'), ...
    'unsupportedTable', 'unsupportedDatetime', 'unsupportedCategorical', ...
    'unsupportedClassdef', '-v7.3');
end
