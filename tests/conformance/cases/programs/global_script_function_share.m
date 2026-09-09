global openmat_global_shared_value
openmat_global_shared_value = 5;
before = openmat_global_shared_value;
from_function = openmat_global_script_function_update();
after = openmat_global_shared_value;
openmat_result = [before, from_function, after];

function value = openmat_global_script_function_update()
global openmat_global_shared_value
openmat_global_shared_value = openmat_global_shared_value + 6;
value = openmat_global_shared_value;
end
