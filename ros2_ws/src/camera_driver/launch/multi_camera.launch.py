from launch import LaunchDescription
from launch.actions import DeclareLaunchArgument, OpaqueFunction
from launch.substitutions import LaunchConfiguration
from launch_ros.actions import Node
from ament_index_python.packages import get_package_share_directory
import os
import yaml


def launch_cameras(context, *args, **kwargs):
    config_file = LaunchConfiguration("config_file").perform(context)

    with open(config_file, "r", encoding="utf-8") as stream:
        config = yaml.safe_load(stream) or {}

    cameras = config.get("cameras", {})
    nodes = []
    for name, params in cameras.items():
        nodes.append(
            Node(
                package="camera_driver",
                executable="camera_driver_node",
                name=name,
                parameters=[params],
                output="screen",
            )
        )

    return nodes


def generate_launch_description():

    config = os.path.join(
        get_package_share_directory("camera_driver"),
        "config",
        "cameras.yaml"
    )

    return LaunchDescription([
        DeclareLaunchArgument(
            "config_file",
            default_value=config,
            description="Path to the camera YAML configuration file.",
        ),
        OpaqueFunction(function=launch_cameras),
    ])