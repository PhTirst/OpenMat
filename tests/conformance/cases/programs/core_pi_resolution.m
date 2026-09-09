bare_value = pi;
functional_value = pi();

pi = 7;
shadowed_bare_value = pi;
shadowed_apply_value = pi();

clear pi;
restored_bare_value = pi;

openmat_result = [bare_value, functional_value, shadowed_bare_value, ...
    shadowed_apply_value, restored_bare_value];
