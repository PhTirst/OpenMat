original = {struct('count', 1, 'nested', {{10, 20}}), {30, 40}};
alias = original;

alias{1}.count = 99;
alias{1}.nested{2} = 77;
alias{2}{1} = 88;

openmat_result = {original, alias};
