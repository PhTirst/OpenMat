original = OpenMatValueCounter(5);
copy = original;
original = original.bump(1);
openmat_result = [original.Value, copy.Value];
