% Minimal package loading/topology check; run from either OpenMat or MATLAB.
ref = fem.ReferenceCell('triangle');
assert(ref.NumVertices == 3);
mesh = fem.Mesh([0, 1, 0, 1; 0, 0, 1, 1], ...
    {'triangle'; 'triangle'}, [1; 4; 7], [1; 2; 3; 2; 4; 3]);
assert(mesh.NumEdges == 5);
assert(numel(mesh.boundaryEdges()) == 4);
disp('FEM topology smoke passed');
