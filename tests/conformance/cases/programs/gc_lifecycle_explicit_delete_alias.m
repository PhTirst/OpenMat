case_id = 6;
openmat_gc_lifecycle_reset(case_id);

object = OpenMatGcLifecycleHandle(case_id, 1);
alias = object;
openmat_gc_lifecycle_append(case_id, 10);
delete(object);
openmat_gc_lifecycle_append(case_id, 11);
object_valid = isvalid(object);
alias_valid = isvalid(alias);
delete(alias);
openmat_gc_lifecycle_append(case_id, 12);

openmat_result = [openmat_gc_lifecycle_read(case_id), ...
    90, double(object_valid), double(alias_valid)];
