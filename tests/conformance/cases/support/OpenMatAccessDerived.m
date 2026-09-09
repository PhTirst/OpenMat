classdef OpenMatAccessDerived < OpenMatAccessBase
    methods
        function object = setProtected(object, value)
            object.ProtectedValue = value;
        end

        function values = readProtected(object)
            values = [object.ProtectedValue, object.protectedMethod()];
        end
    end
end
