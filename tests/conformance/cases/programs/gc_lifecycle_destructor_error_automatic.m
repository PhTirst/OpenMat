case_id = 13;
openmat_gc_lifecycle_reset(case_id);
lastwarn('');

object = OpenMatGcLifecycleHandle(case_id, 1);
object.ThrowOnDelete = true;
openmat_gc_lifecycle_append(case_id, 10);
caught = false;
try
    clear object
    openmat_gc_lifecycle_append(case_id, 11);
catch
    caught = true;
    openmat_gc_lifecycle_append(case_id, 12);
end
[warning_text, ~] = lastwarn;
warning_seen = ~isempty(warning_text);
openmat_gc_lifecycle_append(case_id, 13);

openmat_result = [openmat_gc_lifecycle_read(case_id), 90, ...
    double(caught), double(warning_seen)];
