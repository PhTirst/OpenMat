value_object = OpenMatValueCounter(1);
value_handle = @() value_object.Value;
value_object.Value = 9;

handle_object = OpenMatHandleCounter(2);
identity_handle = @() handle_object.Value;
handle_object.Value = 8;

openmat_result = [value_handle(), identity_handle()];
