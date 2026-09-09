classdef OpenMatGcLifecycleHandle < handle
    properties
        CaseId
        Id
        Peer = []
        RescueOnDelete = false
        ThrowOnDelete = false
        LogDeleteBatchSize = false
    end

    methods
        function object = OpenMatGcLifecycleHandle(case_id, id, fail_construction)
            if nargin < 3
                fail_construction = false;
            end
            object.CaseId = case_id;
            object.Id = id;
            openmat_gc_lifecycle_append(case_id, 100 + id);
            if fail_construction
                error('OpenMatGcLifecycle:ConstructionFailure', ...
                    'OpenMat lifecycle probe construction failure.');
            end
        end

        function delete(objects)
            if objects(1).LogDeleteBatchSize
                openmat_gc_lifecycle_append( ...
                    objects(1).CaseId, 500 + numel(objects));
            end
            for index = 1:numel(objects)
                object = objects(index);
                case_id = object.CaseId;
                id = object.Id;
                openmat_gc_lifecycle_append(case_id, 200 + id);
                openmat_gc_lifecycle_append(case_id, ...
                    300 + 10 * double(isvalid(object)) + id);

                if object.RescueOnDelete
                    global OPENMAT_GC_LIFECYCLE_RESCUED
                    OPENMAT_GC_LIFECYCLE_RESCUED = object;
                    openmat_gc_lifecycle_append(case_id, ...
                        400 + 10 * double(isvalid( ...
                        OPENMAT_GC_LIFECYCLE_RESCUED)) + id);
                end

                if object.ThrowOnDelete
                    error('OpenMatGcLifecycle:DestructorFailure', ...
                        'OpenMat lifecycle probe destructor failure.');
                end
            end
        end
    end
end
