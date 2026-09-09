case_id = 7;
openmat_gc_lifecycle_reset(case_id);
global OPENMAT_GC_LIFECYCLE_RESCUED
OPENMAT_GC_LIFECYCLE_RESCUED = [];

object = OpenMatGcLifecycleHandle(case_id, 1);
object.RescueOnDelete = true;
openmat_gc_lifecycle_append(case_id, 10);
clear object
openmat_gc_lifecycle_append(case_id, 11);

has_rescued_alias = ~isempty(OPENMAT_GC_LIFECYCLE_RESCUED);
rescued_alias_valid = has_rescued_alias && ...
    isvalid(OPENMAT_GC_LIFECYCLE_RESCUED);
property_access_succeeded = false;
try
    rescued_id = OPENMAT_GC_LIFECYCLE_RESCUED.Id; %#ok<NASGU>
    property_access_succeeded = true;
catch
end

openmat_result = [openmat_gc_lifecycle_read(case_id), 90, ...
    double(has_rescued_alias), double(rescued_alias_valid), ...
    double(property_access_succeeded)];
clear global OPENMAT_GC_LIFECYCLE_RESCUED
