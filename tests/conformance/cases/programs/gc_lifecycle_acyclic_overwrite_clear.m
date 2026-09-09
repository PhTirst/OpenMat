case_id = 1;
openmat_gc_lifecycle_reset(case_id);

object = OpenMatGcLifecycleHandle(case_id, 1);
openmat_gc_lifecycle_append(case_id, 10);
object = OpenMatGcLifecycleHandle(case_id, 2);
openmat_gc_lifecycle_append(case_id, 11);
clear object
openmat_gc_lifecycle_append(case_id, 12);

openmat_result = openmat_gc_lifecycle_read(case_id);
