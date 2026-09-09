values = [1, 2; 3, 4];
scaled = oex_scale_in_place(values, 2);
callbackScaled = oex_apply1(@(value) 3 * value, values);
triplet = oex_triplet_demo();

assembler = OexPatternAssembler(3);
initialScale = assembler.Scale;
assembler.Scale = 4;
matrix = assembler.assemble();

copy = assembler.scaledCopy(0.5);
copyScale = oex_assembler_scale(copy);
