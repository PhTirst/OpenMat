classdef OpenMatGcLifecycleValue
    properties
        CaseId
        Id
    end

    methods
        function object = OpenMatGcLifecycleValue(case_id, id)
            object.CaseId = case_id;
            object.Id = id;
            openmat_gc_lifecycle_append(case_id, 100 + id);
        end

        function delete(objects)
            for index = 1:numel(objects)
                openmat_gc_lifecycle_append( ...
                    objects(index).CaseId, 200 + objects(index).Id);
            end
        end
    end
end
