catch_slot = 17;
original_value = catch_slot;
object_inside_handler = false;
try
    value = 1;
    value(2);
catch catch_slot
    object_inside_handler = isobject(catch_slot);
end
openmat_result = [original_value, object_inside_handler, ...
    isobject(catch_slot), numel(catch_slot)];
