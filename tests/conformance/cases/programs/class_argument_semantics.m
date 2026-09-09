value_object = OpenMatValueCounter(5);
handle_object = OpenMatHandleCounter(5);
openmat_mutate_class_arguments(value_object, handle_object, 4);
openmat_result = [value_object.Value, handle_object.Value];
