case_id = 12;
openmat_gc_lifecycle_reset(case_id);
lastwarn('');

object = OpenMatGcLifecycleHandle(case_id, 1);
object.ThrowOnDelete = true;
openmat_gc_lifecycle_append(case_id, 10);
caught = false;
try
    delete(object);
catch
    caught = true;
end
openmat_gc_lifecycle_append(case_id, 11);
valid_after_error = isvalid(object);
[warning_text, ~] = lastwarn;
warning_seen = ~isempty(warning_text);

if valid_after_error
    object.ThrowOnDelete = false;
    clear object
    openmat_gc_lifecycle_append(case_id, 12);
else
    openmat_gc_lifecycle_append(case_id, 13);
end

openmat_result = [openmat_gc_lifecycle_read(case_id), 90, ...
    double(caught), double(valid_after_error), double(warning_seen)];
