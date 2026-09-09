function physical = mapBasis(element, reference, geometry)
%MAPBASIS Map basis values and scalar/componentwise identity gradients.
% Piola values are supported; derivatives of Piola fields require additional
% geometry derivatives and are explicitly not inferred by this helper.
    nq = size(reference.Values, 3);
    if numel(geometry.DetJ) ~= nq
        error('fem:InvalidMapping', 'Basis and geometry point counts differ.');
    end
    physical = reference;
    if strcmp(element.MapType, 'identity')
        if fem.internal.hasField(reference, 'Gradients')
            for q = 1:nq
                J = geometry.Jacobians(:, :, q);
                for c = 1:element.ValueSize
                    gradient = reshape(reference.Gradients(:, c, :, q), [element.NumDofs, 2]);
                    physical.Gradients(:, c, :, q) = reshape(gradient / J, [element.NumDofs, 1, 2]);
                end
            end
        end
    else
        if element.ValueSize ~= 2 || fem.internal.hasField(reference, 'Gradients')
            error('fem:UnsupportedMapping', 'Piola helper supports two-component values without derivatives.');
        end
        for q = 1:nq
            J = geometry.Jacobians(:, :, q);
            values = reference.Values(:, :, q);
            if strcmp(element.MapType, 'covariant')
                physical.Values(:, :, q) = values / J;
            else
                physical.Values(:, :, q) = values * J.' / geometry.DetJ(q);
            end
        end
    end
end
