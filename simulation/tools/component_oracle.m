function component_oracle(output_directory)
% Only OpenMat-authored programs and equations are executed here.
assert(strcmp(version('-release'), '2022b'));
assert(~isfile(fullfile(output_directory, 'components.json')));
inputs = [-3; 0; 3];
values = zeros(3, 1);
for k = 1:3
    values(k) = component_subset(0, [], [], inputs(k), []);
end
sample_time = 0.1;
x = [0; 0];
q = [0; 0];
pending = pi_update(q, 1);
trajectory = zeros(201, 3);
trajectory(1, :) = [0, q(2), x(1)];
for k = 1:200
    % Exact transition of x'' + 2*x' + x = held_command.
    decay = exp(-sample_time);
    position = q(2) + decay*((x(1)-q(2))*(1+sample_time) + x(2)*sample_time);
    velocity = decay*(x(2)*(1-sample_time) - (x(1)-q(2))*sample_time);
    x = [position; velocity];
    q = pending;
    trajectory(k+1, :) = [k*sample_time, q(2), x(1)];
    pending = pi_update(q, 1-x(1));
end
file = fopen(fullfile(output_directory, 'components.json'), 'w');
assert(file >= 0);
cleanup = onCleanup(@() fclose(file));
fprintf(file, '%s', jsonencode(struct('release',version('-release'), ...
    'inputs',inputs,'values',values,'trajectory',trajectory)));
end

function z = pi_update(q, error)
integral = q(1) + 0.8*0.1*error;
command = 1.2*error + integral;
if command > 6
    command = 6;
    integral = q(1);
elseif command < -6
    command = -6;
    integral = q(1);
end
z = [integral; command];
end
