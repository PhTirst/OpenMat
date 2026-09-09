classdef OpenMatEnumRetry
    properties
        Value = 0
    end

    methods
        function object = OpenMatEnumRetry(value)
            global openmat_enum_retry_fail openmat_enum_retry_count
            openmat_enum_retry_count = openmat_enum_retry_count + 1;
            if openmat_enum_retry_fail
                error('OpenMat:EnumRetry', 'OpenMat enum retry probe');
            end
            object.Value = value;
        end
    end

    enumeration
        Only(23)
    end
end
