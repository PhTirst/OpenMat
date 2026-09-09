classdef OpenMatAccessBase
    properties
        PublicValue = 1
    end

    properties (Access = protected)
        ProtectedValue = 2
    end

    properties (Access = private)
        PrivateValue = 3
    end

    methods
        function values = readPrivate(object)
            values = [object.PrivateValue, object.privateMethod()];
        end
    end

    methods (Access = protected)
        function value = protectedMethod(object)
            value = object.ProtectedValue * 2;
        end
    end

    methods (Access = private)
        function value = privateMethod(object)
            value = object.PrivateValue * 2;
        end
    end
end
