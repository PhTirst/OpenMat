original = OpenMatHandleCounter(5);
alias = original;
alias.bump(1);
openmat_result = [original.Value, alias.Value];
