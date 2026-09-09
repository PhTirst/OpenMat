classdef OpenMatAccessProbe
    properties
        PublicValue
    end

    properties (Access = private)
        SecretValue
    end

    methods
        function object = OpenMatAccessProbe(publicValue, secretValue)
            object.PublicValue = publicValue;
            object.SecretValue = secretValue;
        end

        function value = revealSecret(object)
            value = object.SecretValue;
        end
    end
end
