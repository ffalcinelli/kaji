import re
import glob

def fix_file(path):
    with open(path, "r") as f:
        content = f.read()

    # Find plan::run(
    idx = 0
    while True:
        idx = content.find("plan::run(", idx)
        if idx == -1:
            break

        start_idx = idx

        # Parse the arguments manually counting parentheses
        idx += len("plan::run(")

        paren_count = 1
        arg_starts = [idx]
        args = []

        in_string = False
        escape = False

        current_idx = idx
        while current_idx < len(content):
            char = content[current_idx]

            if escape:
                escape = False
            elif char == '\\':
                escape = True
            elif char == '"':
                in_string = not in_string
            elif not in_string:
                if char == '(':
                    paren_count += 1
                elif char == ')':
                    paren_count -= 1
                    if paren_count == 0:
                        args.append(content[arg_starts[-1]:current_idx])
                        break
                elif char == ',' and paren_count == 1:
                    args.append(content[arg_starts[-1]:current_idx])
                    arg_starts.append(current_idx + 1)

            current_idx += 1

        end_idx = current_idx + 1

        # If we successfully parsed 8 args, replace it
        # Sometimes there might be a trailing comma making it 9 arguments where the last is empty
        args = [arg.strip() for arg in args]
        if len(args) > 0 and args[-1] == "":
            args.pop()

        if len(args) == 8:
            new_call = f"""plan::run(kaji::plan::PlanArgs {{
        client: {args[0]},
        workspace_dir: {args[1]},
        changes_only: {args[2]},
        interactive: {args[3]},
        realms_to_plan: {args[4]},
        ui: {args[5]},
        resolver: {args[6]},
        profile: {args[7]},
    }})"""
            content = content[:start_idx] + new_call + content[end_idx:]
            idx = start_idx + len(new_call)
        else:
            idx = start_idx + len("plan::run(")

    with open(path, "w") as f:
        f.write(content)

for path in glob.glob("tests/*.rs"):
    fix_file(path)
