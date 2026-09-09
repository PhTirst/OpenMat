function openmat_gc_lifecycle_append(case_id, event_code)
global OPENMAT_GC_LIFECYCLE_EVENTS

if isempty(OPENMAT_GC_LIFECYCLE_EVENTS)
    OPENMAT_GC_LIFECYCLE_EVENTS = zeros(0, 2);
end
OPENMAT_GC_LIFECYCLE_EVENTS(end + 1, :) = [case_id, event_code];
end
